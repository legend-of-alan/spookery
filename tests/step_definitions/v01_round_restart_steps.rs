use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use cucumber::{then, when};
use spookery_kernel::shard::Shard;

use crate::support::{
    self, pack_arg, spawn, text_of, wait_count, wait_exit, wait_has, wait_listen, write_stdin, Proc,
};
use crate::World;

// The in-memory queue and the live run, gm, and play processes cannot live on World.
struct Live {
    player: Option<Proc>,
    gm: Option<Proc>,
    server: Option<Proc>,
    lock: Option<support::PortLock>,
}

static LIVE: Mutex<Live> = Mutex::new(Live {
    player: None,
    gm: None,
    server: None,
    lock: None,
});

static SHARD: Mutex<Option<Shard>> = Mutex::new(None);

fn live() -> std::sync::MutexGuard<'static, Live> {
    LIVE.lock().unwrap_or_else(|err| err.into_inner())
}

fn shard_slot() -> std::sync::MutexGuard<'static, Option<Shard>> {
    SHARD.lock().unwrap_or_else(|err| err.into_inner())
}

fn pid_file() -> PathBuf {
    std::env::temp_dir().join("spook-bdd-v01-round-restart.pids")
}

fn remember_pids() {
    let held = live();
    let mut text = String::new();
    for proc in [held.server.as_ref(), held.gm.as_ref(), held.player.as_ref()]
        .into_iter()
        .flatten()
    {
        text.push_str(&proc.child.id().to_string());
        text.push('\n');
    }
    drop(held);
    let _ = fs::write(pid_file(), text);
}

fn kill_pid(pid: u32) {
    let listed = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output();
    let Ok(listed) = listed else {
        return;
    };
    let text = String::from_utf8_lossy(&listed.stdout);
    if !text.to_ascii_lowercase().contains("spook") {
        return;
    }
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output();
}

fn kill_remembered() {
    let Ok(text) = fs::read_to_string(pid_file()) else {
        return;
    };
    for line in text.lines() {
        if let Ok(pid) = line.parse::<u32>() {
            kill_pid(pid);
        }
    }
    let _ = fs::remove_file(pid_file());
}

fn clear_live() {
    let mut held = live();
    held.player.take();
    held.gm.take();
    held.server.take();
    held.lock.take();
    drop(held);
    let _ = fs::remove_file(pid_file());
}

fn reclaim() {
    let occupied = {
        let held = live();
        held.server.is_some() || held.gm.is_some() || held.player.is_some() || held.lock.is_some()
    };
    if occupied {
        clear_live();
    } else {
        kill_remembered();
    }
}

fn assign_job(child: &std::process::Child) {
    #[cfg(windows)]
    windows_job::assign(child);
    #[cfg(not(windows))]
    let _ = child;
}

fn pack_of(w: &World) -> &Path {
    w.pack.as_ref().unwrap()
}

fn now_of(pack: &Path) -> spookery_kernel::model::Now {
    spookery_kernel::open_shard(pack)
        .unwrap_or_else(|err| panic!("open: {err}"))
        .now()
        .clone()
}

fn tick_value(pack: &Path) -> serde_json::Value {
    let text = fs::read_to_string(pack.join("state/log/tick-0001.jsonl")).unwrap();
    serde_json::from_str(text.trim()).unwrap()
}

fn player_waits(tick: &serde_json::Value) -> Vec<String> {
    tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .map(|intent| intent["id"].as_str().unwrap().to_string())
        .collect()
}

fn queued_id(stdout: &str) -> String {
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("kind").and_then(|kind| kind.as_str()) == Some("queued") {
            return value
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap()
                .to_string();
        }
    }
    panic!("no queued line in {stdout}");
}

fn scratch_u64(w: &World, key: &str) -> u64 {
    w.scratch
        .get(key)
        .unwrap_or_else(|| panic!("missing {key}"))
        .parse()
        .unwrap()
}

fn stamp(shard: &Shard, w: &mut World) {
    let now = shard.now();
    w.scratch.insert(
        "committed_world_time".to_string(),
        now.world_time.to_string(),
    );
    w.scratch.insert(
        "committed_tick_index".to_string(),
        now.tick_index.to_string(),
    );
    w.scratch.insert(
        "committed_humans".to_string(),
        now.presence.humans.to_string(),
    );
    w.scratch.insert(
        "committed_last_tick_at".to_string(),
        now.last_tick_at.clone(),
    );
    w.scratch
        .insert("committed_state_rev".to_string(), now.state_rev.to_string());
}

#[then(expr = "schedule.rs defines wake and wake_shard without player_held or gm_held")]
fn schedule_wrapper(_w: &mut World) {
    let schedule =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cli/schedule.rs"))
            .unwrap();
    assert!(schedule.contains("pub fn wake("));
    assert!(schedule.contains("pub fn wake_shard("));
    assert!(!schedule.contains("player_held"));
    assert!(!schedule.contains("gm_held"));
}

#[then(expr = "turn.rs calls schedule::wake_shard and does not open a shard")]
fn turn_wrapper(_w: &mut World) {
    let turn =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cli/turn.rs"))
            .unwrap();
    assert!(turn.contains("schedule::wake_shard"));
    assert!(!turn.contains("open_shard"));
}

#[when(expr = "the round restart shard seats pc_rowan")]
fn seat(w: &mut World) {
    let mut shard =
        spookery_kernel::open_shard(pack_of(w)).unwrap_or_else(|err| panic!("open: {err}"));
    let session = spookery::seat::seat_player(&mut shard, "pc_rowan")
        .unwrap_or_else(|err| panic!("seat: {err:?}"));
    w.scratch.insert("session_id".to_string(), session);
    *shard_slot() = Some(shard);
}

#[when(expr = "the round restart shard queues a wait on the open shard")]
fn queue_on_open(w: &mut World) {
    let mut slot = shard_slot();
    let shard = slot.as_mut().unwrap();
    let note = spookery::seat::queue_wait(shard, "pc_rowan")
        .unwrap_or_else(|err| panic!("queue: {err:?}"));
    w.scratch.insert("queued_id".to_string(), note.id);
}

#[when(expr = "the round restart shard opens and queues a wait for pc_rowan")]
fn open_and_queue(w: &mut World) {
    let mut shard =
        spookery_kernel::open_shard(pack_of(w)).unwrap_or_else(|err| panic!("open: {err}"));
    let note = spookery::seat::queue_wait(&mut shard, "pc_rowan")
        .unwrap_or_else(|err| panic!("queue: {err:?}"));
    w.scratch.insert("queued_id".to_string(), note.id);
    *shard_slot() = Some(shard);
}

#[when(expr = "the round restart shard commits the queue")]
fn commit_queue(w: &mut World) {
    let mut slot = shard_slot();
    let shard = slot.as_mut().unwrap();
    let advanced =
        spookery::seat::commit_queued(shard).unwrap_or_else(|err| panic!("commit: {err:?}"));
    assert!(advanced);
    stamp(shard, w);
}

#[then(expr = "the committed world_time is {int}")]
fn committed_world_time(w: &mut World, n: u64) {
    assert_eq!(scratch_u64(w, "committed_world_time"), n);
}

#[then(expr = "the committed tick_index is {int}")]
fn committed_tick_index(w: &mut World, n: u64) {
    assert_eq!(scratch_u64(w, "committed_tick_index"), n);
}

#[then(expr = "the committed humans is {int}")]
fn committed_humans(w: &mut World, n: u64) {
    assert_eq!(scratch_u64(w, "committed_humans"), n);
}

#[then(expr = "the committed last_tick_at is not 2026-01-01T00:00:00Z")]
fn committed_last_tick_at(w: &mut World) {
    assert_ne!(
        w.scratch.get("committed_last_tick_at").unwrap(),
        "2026-01-01T00:00:00Z"
    );
}

#[when(expr = "the round restart player seat closes")]
fn seat_closes(w: &mut World) {
    let session = w.scratch.get("session_id").unwrap().clone();
    let mut slot = shard_slot();
    let shard = slot.as_mut().unwrap();
    spookery::seat::release_player_seat(
        shard,
        &session,
        "pc_rowan",
        spookery::seat::SocketEnd::Close,
    )
    .unwrap_or_else(|err| panic!("release: {err:?}"));
}

#[then(expr = "round restart humans is {int}")]
fn humans_is(w: &mut World, n: u64) {
    assert_eq!(u64::from(now_of(pack_of(w)).presence.humans), n);
}

#[then(expr = "the round restart queue is empty")]
fn queue_empty(_w: &mut World) {
    let slot = shard_slot();
    assert_eq!(slot.as_ref().unwrap().queued_intents().len(), 0);
}

#[when(expr = "a round restart wake runs")]
fn wake(w: &mut World) {
    let mut slot = shard_slot();
    let shard = slot.as_mut().unwrap();
    let advanced = spookery::seat::on_scheduled_wake(shard, false, false)
        .unwrap_or_else(|err| panic!("wake: {err:?}"));
    w.scratch
        .insert("wake_advanced".to_string(), advanced.to_string());
}

#[then(expr = "that round restart wake did not advance")]
fn wake_stayed(w: &mut World) {
    assert_eq!(w.scratch.get("wake_advanced").unwrap(), "false");
}

#[then(expr = "that round restart wake advanced")]
fn wake_advanced(w: &mut World) {
    assert_eq!(w.scratch.get("wake_advanced").unwrap(), "true");
}

#[then(expr = "world_time is still the committed stamp")]
fn world_time_stamp(w: &mut World) {
    let slot = shard_slot();
    assert_eq!(
        slot.as_ref().unwrap().now().world_time,
        scratch_u64(w, "committed_world_time")
    );
}

#[then(expr = "tick_index is still the committed stamp")]
fn tick_index_stamp(w: &mut World) {
    let slot = shard_slot();
    assert_eq!(
        slot.as_ref().unwrap().now().tick_index,
        scratch_u64(w, "committed_tick_index")
    );
}

#[then(expr = "last_tick_at is still the committed stamp")]
fn last_tick_stamp(w: &mut World) {
    let slot = shard_slot();
    assert_eq!(
        &slot.as_ref().unwrap().now().last_tick_at,
        w.scratch.get("committed_last_tick_at").unwrap()
    );
}

#[then(expr = "state_rev is one more than the committed stamp")]
fn state_rev_plus(w: &mut World) {
    let slot = shard_slot();
    assert_eq!(
        slot.as_ref().unwrap().now().state_rev,
        scratch_u64(w, "committed_state_rev") + 1
    );
}

#[then(expr = "tick-0002.jsonl player waits are empty")]
fn tick_two_waits(w: &mut World) {
    let text = fs::read_to_string(pack_of(w).join("state/log/tick-0002.jsonl")).unwrap();
    let second: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    let waits = player_waits(&second);
    assert!(waits.is_empty(), "{second}");
}

#[when(expr = "the round restart shard is listening")]
fn listen(w: &mut World) {
    reclaim();
    {
        let mut held = live();
        held.lock = Some(support::lock_port());
    }
    let pack = pack_arg(pack_of(w));
    let mut server = spawn(&["run", &pack]);
    assign_job(&server.child);
    wait_listen(&mut server);
    {
        let mut held = live();
        held.server = Some(server);
    }
    remember_pids();
}

#[when(expr = "a round restart GM client is seated")]
fn gm_seated(w: &mut World) {
    let pack = pack_arg(pack_of(w));
    let gm = spawn(&["gm", &pack]);
    assign_job(&gm.child);
    {
        let mut held = live();
        held.gm = Some(gm);
    }
    remember_pids();
    std::thread::sleep(Duration::from_millis(200));
    let mut held = live();
    let gm = held.gm.as_mut().unwrap();
    if gm.child.try_wait().unwrap().is_some() {
        panic!(
            "gm exited\nstdout:\n{}\nstderr:\n{}",
            text_of(&gm.stdout),
            text_of(&gm.stderr)
        );
    }
}

#[when(expr = "the round restart player connects and perceives")]
fn player_connects(w: &mut World) {
    let pack = pack_arg(pack_of(w));
    let player = spawn(&["play", &pack]);
    assign_job(&player.child);
    let stdout = std::sync::Arc::clone(&player.stdout);
    {
        let mut held = live();
        held.player = Some(player);
    }
    remember_pids();
    wait_has(&stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
}

#[when(expr = "the round restart player sends wait")]
fn player_wait(w: &mut World) {
    let (stdout, before_len) = {
        let mut held = live();
        let player = held.player.as_mut().unwrap();
        let before = text_of(&player.stdout);
        write_stdin(player, "wait");
        (std::sync::Arc::clone(&player.stdout), before.len())
    };
    let text = wait_has(&stdout, "\"kind\":\"queued\"", Duration::from_secs(5));
    let added = text[before_len..].to_string();
    let id = queued_id(&text);
    let stderr = {
        let held = live();
        text_of(&held.player.as_ref().unwrap().stderr)
    };
    w.scratch.insert("added_stdout".to_string(), added);
    w.scratch.insert("queued_id".to_string(), id);
    w.scratch.insert("player_stderr".to_string(), stderr);
}

#[then(expr = "the round restart player gained a queued line and not a resume")]
fn queued_not_resume(w: &mut World) {
    let added = w.scratch.get("added_stdout").unwrap();
    assert!(added.contains("\"kind\":\"queued\""), "{added}");
    assert_eq!(added.matches("\"kind\":\"resume\"").count(), 0, "{added}");
}

#[then(expr = "the round restart player stderr does not contain NOT_IN_SPEC")]
fn stderr_ok(w: &mut World) {
    let stderr = w.scratch.get("player_stderr").unwrap();
    assert!(!stderr.contains("NOT_IN_SPEC"), "{stderr}");
}

#[when(expr = "the round restart player says before the round")]
fn player_says(w: &mut World) {
    let (stdout, before_len) = {
        let mut held = live();
        let player = held.player.as_mut().unwrap();
        let before = text_of(&player.stdout);
        write_stdin(player, "say before the round");
        (std::sync::Arc::clone(&player.stdout), before.len())
    };
    let text = wait_has(&stdout, "before the round", Duration::from_secs(5));
    w.scratch
        .insert("added_stdout".to_string(), text[before_len..].to_string());
}

#[then(expr = "the round restart player gained a say line and not a resume")]
fn say_not_resume(w: &mut World) {
    let added = w.scratch.get("added_stdout").unwrap();
    assert!(added.contains("\"kind\":\"say\""), "{added}");
    assert!(!added.contains("\"kind\":\"resume\""), "{added}");
}

#[then(expr = "session.jsonl exists")]
fn session_exists(w: &mut World) {
    assert!(pack_of(w).join("state/log/session.jsonl").is_file());
}

#[when(expr = "the round restart GM closes")]
fn gm_closes(w: &mut World) {
    let mut held = live();
    let gm = held.gm.as_mut().unwrap();
    write_stdin(gm, "close");
    let status = wait_exit(gm, Duration::from_secs(5));
    w.scratch.insert(
        "gm_exit".to_string(),
        status.code().unwrap_or(-1).to_string(),
    );
}

#[then(expr = "the round restart GM exits 0")]
fn gm_exits_0(w: &mut World) {
    assert_eq!(w.scratch.get("gm_exit").unwrap(), "0");
}

#[when(expr = "the round restart player sends wait again")]
fn player_wait_again(w: &mut World) {
    let stderr = {
        let mut held = live();
        let player = held.player.as_mut().unwrap();
        write_stdin(player, "wait");
        std::sync::Arc::clone(&player.stderr)
    };
    let text = wait_has(&stderr, "SCHEMA", Duration::from_secs(5));
    w.scratch.insert("player_stderr".to_string(), text);
}

#[then(expr = "the round restart player stderr is SCHEMA")]
fn stderr_schema(w: &mut World) {
    let text = w.scratch.get("player_stderr").unwrap();
    assert_eq!(text.trim(), "SCHEMA");
}

#[then(expr = "the round restart player gained a resume")]
fn gained_resume(w: &mut World) {
    let added = w.scratch.get("added_stdout").unwrap();
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
}

#[then(expr = "tick-0001.jsonl has both queued waits")]
fn both_waits(w: &mut World) {
    let tick = tick_value(pack_of(w));
    let waits = player_waits(&tick);
    assert_eq!(waits.len(), 2, "{tick}");
    assert_eq!(&waits[0], w.scratch.get("queued_id").unwrap());
    assert_ne!(waits[0], waits[1]);
}

#[then(expr = "the round restart processes stop")]
fn stop(_w: &mut World) {
    clear_live();
    *shard_slot() = None;
}

#[cfg(windows)]
mod windows_job {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::sync::OnceLock;

    #[repr(C)]
    struct BasicLimit {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        _pad0: u32,
        min_ws: usize,
        max_ws: usize,
        active_process_limit: u32,
        _pad1: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_op: u64,
        write_op: u64,
        other_op: u64,
        read_xfer: u64,
        write_xfer: u64,
        other_xfer: u64,
    }

    #[repr(C)]
    struct ExtendedLimit {
        basic: BasicLimit,
        io: IoCounters,
        process_memory: usize,
        job_memory: usize,
        peak_process: usize,
        peak_job: usize,
    }

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attrs: *mut c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            info: *const c_void,
            len: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }

    fn job() -> *mut c_void {
        static JOB: OnceLock<usize> = OnceLock::new();
        let raw = *JOB.get_or_init(|| {
            let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
            assert!(!handle.is_null(), "CreateJobObjectW failed: {}", unsafe {
                GetLastError()
            });
            let mut info = ExtendedLimit {
                basic: BasicLimit {
                    per_process_user_time: 0,
                    per_job_user_time: 0,
                    limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    _pad0: 0,
                    min_ws: 0,
                    max_ws: 0,
                    active_process_limit: 0,
                    _pad1: 0,
                    affinity: 0,
                    priority_class: 0,
                    scheduling_class: 0,
                },
                io: IoCounters {
                    read_op: 0,
                    write_op: 0,
                    other_op: 0,
                    read_xfer: 0,
                    write_xfer: 0,
                    other_xfer: 0,
                },
                process_memory: 0,
                job_memory: 0,
                peak_process: 0,
                peak_job: 0,
            };
            let ok = unsafe {
                SetInformationJobObject(
                    handle,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    (&mut info as *mut ExtendedLimit).cast(),
                    std::mem::size_of::<ExtendedLimit>() as u32,
                )
            };
            assert!(ok != 0, "SetInformationJobObject failed: {}", unsafe {
                GetLastError()
            });
            handle as usize
        });
        raw as *mut c_void
    }

    pub fn assign(child: &Child) {
        let ok = unsafe { AssignProcessToJobObject(job(), child.as_raw_handle()) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err != 5 {
                panic!("AssignProcessToJobObject failed: {err}");
            }
        }
    }
}
