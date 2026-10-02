use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::Duration;

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, text_of, wait_has, write_stdin, Proc};
use crate::World;

// Procs cannot live on World. One scenario runs at a time.
struct Session {
    server: Proc,
    player: Proc,
    stdout_before: String,
    pack: PathBuf,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn lock_session() -> std::sync::MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|err| err.into_inner())
}

fn stop_proc(proc: &mut Proc) {
    let _ = proc.child.kill();
    let _ = proc.child.wait();
}

fn drop_session() {
    let session = lock_session().take();
    let Some(mut session) = session else {
        return;
    };
    stop_proc(&mut session.player);
    stop_proc(&mut session.server);
    let pack = session.pack.clone();
    drop(session);
    let _ = fs::remove_dir_all(pack);
}

fn current_pack() -> PathBuf {
    lock_session()
        .as_ref()
        .expect("no lists shard")
        .pack
        .clone()
}

fn now_of(pack: &Path) -> spookery_kernel::model::Now {
    spookery_kernel::open_shard(pack)
        .unwrap_or_else(|err| panic!("open: {err}"))
        .now()
        .clone()
}

fn current_now() -> spookery_kernel::model::Now {
    now_of(&current_pack())
}

fn rowan_place(now: &spookery_kernel::model::Now) -> String {
    now.actors
        .get("pc_rowan")
        .unwrap_or_else(|| panic!("pc_rowan missing"))
        .place()
        .to_string()
}

fn flags_of(now: &spookery_kernel::model::Now, id: &str) -> Vec<String> {
    match now.actors.get(id) {
        Some(
            spookery_kernel::model::ActorLive::Npc { flags, .. }
            | spookery_kernel::model::ActorLive::Pc { flags, .. },
        ) => flags.clone(),
        None => panic!("{id} missing"),
    }
}

fn tick_names(pack: &Path) -> Vec<String> {
    let log = pack.join("state/log");
    if !log.is_dir() {
        return Vec::new();
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&log).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        if name.starts_with("tick-") && name.ends_with(".jsonl") {
            names.push(name.into_owned());
        }
    }
    names.sort();
    names
}

fn tick_value(pack: &Path, name: &str) -> Value {
    let text = fs::read_to_string(pack.join("state/log").join(name)).unwrap();
    serde_json::from_str(text.trim()).unwrap()
}

fn flag_effects(tick: &Value) -> Vec<&Value> {
    tick["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| effect["class"] == "flag")
        .collect()
}

fn assert_no_derived_row(tick: &Value, moves: &[&str]) {
    let tick_index = tick["tick_index"].as_u64().unwrap();
    let intents = tick["intents"].as_array().unwrap();
    if let Some(sample) = intents.iter().find(|intent| intent["source"] == "kernel") {
        let actor = sample["actor_id"].as_str().unwrap();
        let move_name = sample["move"].as_str().unwrap();
        let expect =
            spookery_kernel::derive::kernel_intent_id("docks-001", tick_index, actor, move_name);
        assert_eq!(sample["id"].as_str().unwrap(), expect, "{sample}");
    }
    for actor in ["mira", "bram", "aldric", "pc_rowan"] {
        for move_name in moves {
            let id = spookery_kernel::derive::kernel_intent_id(
                "docks-001",
                tick_index,
                actor,
                move_name,
            );
            assert!(
                intents.iter().all(|intent| intent["id"] != id),
                "{actor} derived {move_name}"
            );
            assert!(
                tick["rejected"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["id"] != id),
                "{actor} rejected derived {move_name}"
            );
        }
    }
    assert!(intents.iter().all(|intent| {
        intent["source"] != "kernel" || !moves.contains(&intent["move"].as_str().unwrap())
    }));
}

fn stamp_pack_rev(pack: &Path) {
    let text = fs::read_to_string(pack.join("state/now.yaml")).unwrap();
    let mut now = spookery_kernel::codec::load_now(&text).unwrap();
    now.pack_rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    fs::write(
        pack.join("state/now.yaml"),
        spookery_kernel::codec::emit_now(&now).unwrap(),
    )
    .unwrap();
}

fn write_grit(pack: &Path, grit: i64) {
    let sheet = pack.join("state/actors/pc_rowan.yaml");
    let sheet_text = fs::read_to_string(&sheet).unwrap();
    let sheet_next = sheet_text.replacen(
        "stats:\n  grit: 1\n",
        &format!("stats:\n  grit: {grit}\n"),
        1,
    );
    assert!(
        sheet_next.contains(&format!("grit: {grit}\n")),
        "{sheet_text}"
    );
    fs::write(&sheet, sheet_next).unwrap();
}

fn action_pack(label: &str) -> PathBuf {
    let pack = support::temp_pack(label);
    let moves = pack.join("rules/moves.yaml");
    let mut text = fs::read_to_string(&moves).unwrap();
    text.push_str(
        "  - name: mark
    token: actor
    on: named
    effects:
      - class: flag
        key: marked
        value: true
  - name: wave
    token: none
    on: speaker
    effects:
      - class: flag
        key: marked
        value: true
",
    );
    fs::write(&moves, text).unwrap();
    let mira = pack.join("cast/mira.md");
    let mira_text = fs::read_to_string(&mira).unwrap();
    let patched = mira_text.replacen(
        "tick:\n  eligible: true\n",
        "tick:\n  eligible: true\n  flags:\n    - marked\n",
        1,
    );
    assert_ne!(patched, mira_text, "mira tick block was not patched");
    fs::write(&mira, patched).unwrap();
    stamp_pack_rev(&pack);
    pack
}

fn check_pack(label: &str, grit: i64) -> PathBuf {
    let pack = action_pack(label);
    let moves = pack.join("rules/moves.yaml");
    let text = fs::read_to_string(&moves).unwrap();
    let next = text.replacen(
        "  - name: mark\n    token: actor\n    on: named\n    effects:\n",
        "  - name: mark\n    token: actor\n    on: named\n    check: grit\n    effects:\n",
        1,
    );
    assert_ne!(next, text, "mark row was not given a check");
    fs::write(&moves, next).unwrap();
    write_grit(&pack, grit);
    let canon = spookery_kernel::canon::load_canon(&pack).unwrap();
    assert_eq!(
        canon.moves.get("mark").unwrap().check.as_deref(),
        Some("grit")
    );
    stamp_pack_rev(&pack);
    pack
}

fn outcome_pack(label: &str, grit: i64) -> PathBuf {
    let pack = support::temp_pack(label);
    let moves = pack.join("rules/moves.yaml");
    let mut text = fs::read_to_string(&moves).unwrap();
    text.push_str(OUTCOME_ROW);
    fs::write(&moves, text).unwrap();
    let mira = pack.join("cast/mira.md");
    let mira_text = fs::read_to_string(&mira).unwrap();
    let patched = mira_text.replacen(
        "tick:\n  eligible: true\n",
        "tick:\n  eligible: true\n  flags:\n    - marked\n    - missed\n",
        1,
    );
    assert_ne!(patched, mira_text, "mira tick block was not patched");
    fs::write(&mira, patched).unwrap();
    write_grit(&pack, grit);
    stamp_pack_rev(&pack);
    pack
}

const OUTCOME_ROW: &str = "  - name: mark
    token: actor
    on: named
    check: grit
    success:
      - class: flag
        key: marked
        value: true
    failure:
      - class: flag
        key: missed
        value: true
";

const ATTACK_ROW: &str = "  - name: attack
    token: actor
    on: named
    check: grit
    success:
      - class: resource
        key: hp
        delta: -1
    failure:
      - class: resource
        key: hp
        delta: 0
";

fn seat_player(pack: &Path) -> Proc {
    let player = support::spawn(&["play", &support::pack_arg(pack)]);
    assign_job(&player.child);
    wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    assert!(
        text_of(&player.stderr).is_empty(),
        "{}",
        text_of(&player.stderr)
    );
    player
}

fn sit(w: &mut World) {
    drop_session();
    let pack = w.pack.clone().expect("lists pack");
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let player = seat_player(&pack);
    *lock_session() = Some(Session {
        server,
        player,
        stdout_before: String::new(),
        pack,
    });
}

fn assign_job(child: &Child) {
    #[cfg(windows)]
    windows_job::assign(child);
    #[cfg(not(windows))]
    let _ = child;
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

#[given(expr = "the lists port is locked")]
fn lists_port_locked(w: &mut World) {
    drop_session();
    w.port = Some(crate::PortGuard(support::lock_port()));
}

#[given(expr = "a lists check pack labeled {string} with grit {int}")]
fn lists_check_pack(w: &mut World, label: String, grit: i64) {
    w.pack = Some(check_pack(&label, grit));
}

#[given(expr = "a lists outcome pack labeled {string} with grit {int}")]
fn lists_outcome_pack(w: &mut World, label: String, grit: i64) {
    w.pack = Some(outcome_pack(&label, grit));
}

#[given(
    expr = "the lists attack row loads as token actor, on named, check grit, empty effects, success hp -1, and failure hp 0"
)]
fn lists_attack_row(w: &mut World) {
    let _ = w;
    let pack = support::temp_pack("p11-attack");
    let path = pack.join("rules/moves.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replacen("  - name: attack\n", ATTACK_ROW, 1);
    assert_ne!(next, text, "attack row was not written");
    fs::write(&path, next).unwrap();
    let canon = spookery_kernel::canon::load_canon(&pack).unwrap();
    let attack = canon.moves.get("attack").unwrap();
    assert_eq!(attack.check.as_deref(), Some("grit"));
    let row = attack.row.as_ref().unwrap();
    assert_eq!(row.token, spookery_kernel::moves::MoveToken::Actor);
    assert_eq!(row.on, spookery_kernel::moves::MoveOn::Named);
    assert!(row.effects.is_empty());
    assert_eq!(
        row.success,
        vec![spookery_kernel::moves::RowEffect::Resource {
            key: "hp".to_string(),
            delta: -1,
        }]
    );
    assert_eq!(
        row.failure,
        vec![spookery_kernel::moves::RowEffect::Resource {
            key: "hp".to_string(),
            delta: 0,
        }]
    );
    let _ = fs::remove_dir_all(&pack);
}

#[when(expr = "a lists player sits with no GM")]
fn lists_sits(w: &mut World) {
    sit(w);
}

#[when(expr = "the lists player marks mira")]
fn lists_mark(w: &mut World) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no lists shard");
    session.stdout_before = text_of(&session.player.stdout);
    write_stdin(&mut session.player, "mark mira");
}

#[then(expr = "the lists mark row names check grit")]
fn lists_mark_names_grit(w: &mut World) {
    let pack = w.pack.as_ref().expect("lists pack");
    let canon = spookery_kernel::canon::load_canon(pack).unwrap();
    assert_eq!(
        canon.moves.get("mark").unwrap().check.as_deref(),
        Some("grit")
    );
}

#[then(
    expr = "the lists mark row names check grit with empty effects, success flag marked, and failure flag missed"
)]
fn lists_mark_outcome_row(w: &mut World) {
    let pack = w.pack.as_ref().expect("lists pack");
    let canon = spookery_kernel::canon::load_canon(pack).unwrap();
    let mark = canon.moves.get("mark").unwrap();
    assert_eq!(mark.check.as_deref(), Some("grit"));
    let row = mark.row.as_ref().unwrap();
    assert!(row.effects.is_empty());
    assert_eq!(
        row.success,
        vec![spookery_kernel::moves::RowEffect::Flag {
            key: "marked".to_string(),
            value: true,
        }]
    );
    assert_eq!(
        row.failure,
        vec![spookery_kernel::moves::RowEffect::Flag {
            key: "missed".to_string(),
            value: true,
        }]
    );
}

#[then(expr = "lists humans is {int}")]
fn lists_humans(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().presence.humans as i64, n);
}

#[then(expr = "lists world_time is {int}")]
fn lists_world_time(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().world_time, n as u64);
}

#[then(expr = "lists tick_index is {int}")]
fn lists_tick_index(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().tick_index, n as u64);
}

#[then(expr = "lists Rowan is in {word}")]
fn lists_rowan(w: &mut World, place: String) {
    let _ = w;
    assert_eq!(rowan_place(&current_now()), place);
}

#[then(expr = "lists actor {word} has no flags")]
fn lists_actor_empty(w: &mut World, id: String) {
    let _ = w;
    assert!(flags_of(&current_now(), &id).is_empty(), "{id}");
}

#[then(expr = "lists actor {word} flags are {word}")]
fn lists_actor_flags(w: &mut World, id: String, flag: String) {
    let _ = w;
    assert_eq!(flags_of(&current_now(), &id), vec![flag]);
}

#[then(expr = "lists tick files are none")]
fn lists_no_ticks(w: &mut World) {
    let _ = w;
    assert!(tick_names(&current_pack()).is_empty());
}

#[then(expr = "the lists player sees check success {word} with a resume and no faces")]
fn lists_sees_check(w: &mut World, word: String) {
    let _ = w;
    let line = format!("{{\"kind\":\"check\",\"success\":{word}}}");
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no lists shard");
    let stdout = wait_has(&session.player.stdout, &line, Duration::from_secs(5));
    let added = &stdout[session.stdout_before.len()..];
    assert!(added.contains(&line), "{added}");
    assert!(!added.contains("\"faces\""), "{added}");
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
}

#[then(expr = "the lists player stderr is empty")]
fn lists_stderr_empty(w: &mut World) {
    let _ = w;
    let session = lock_session();
    let session = session.as_ref().expect("no lists shard");
    assert!(
        text_of(&session.player.stderr).is_empty(),
        "{}",
        text_of(&session.player.stderr)
    );
}

#[then(expr = "lists tick {string} world_time goes from {int} to {int}")]
fn lists_tick_span(w: &mut World, name: String, before: i64, after: i64) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    assert_eq!(tick["world_time_before"], before);
    assert_eq!(tick["world_time_after"], after);
}

#[then(expr = "lists tick {string} traces resolve_check")]
fn lists_traces_check(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    assert!(tick["tool_trace"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry == "resolve_check"));
}

#[then(expr = "lists tick {string} has one player mark intent targeting mira")]
fn lists_mark_intent(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    let marks: Vec<_> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "mark")
        .collect();
    assert_eq!(marks.len(), 1, "{tick}");
    assert_eq!(marks[0]["source"], "player");
    assert_eq!(marks[0]["actor_id"], "pc_rowan");
    assert_eq!(marks[0]["target"]["actor"], "mira");
}

#[then(expr = "lists tick {string} has one flag effect on mira key {word}")]
fn lists_flag_effect(w: &mut World, name: String, key: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    let marks: Vec<_> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "mark")
        .collect();
    assert_eq!(marks.len(), 1, "{tick}");
    let flags = flag_effects(&tick);
    assert_eq!(flags.len(), 1, "{tick}");
    assert_eq!(flags[0]["intent_id"], marks[0]["id"]);
    assert_eq!(flags[0]["as"], "player:pc_rowan");
    assert_eq!(flags[0]["payload"]["actor_id"], "mira");
    assert_eq!(flags[0]["payload"]["key"], key);
    assert_eq!(flags[0]["payload"]["value"], true);
}

#[then(expr = "lists tick {string} derives neither mark nor wave")]
fn lists_no_derived(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    assert_no_derived_row(&tick, &["mark", "wave"]);
}

#[then(expr = "the lists session is closed")]
fn lists_closed(w: &mut World) {
    let _ = w;
    drop_session();
}
