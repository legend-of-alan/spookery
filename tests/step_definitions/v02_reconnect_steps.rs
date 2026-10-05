use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, text_of, wait_count, wait_exit, wait_has, write_stdin, Proc};
use crate::World;

// Procs and the GM socket cannot live on World. One scenario runs at a time.
// Fields drop in declaration order: the player, the socket, then the server.
// The port lock stays on World and drops after that.
struct Session {
    player: Option<Proc>,
    gm: TcpStream,
    server: Proc,
    next_id: u64,
    pack: PathBuf,
}

struct Record {
    wait_id: Option<String>,
    travel_id: Option<String>,
    waited_stdout: String,
    commit: Option<Value>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static RECORD: Mutex<Record> = Mutex::new(Record {
    wait_id: None,
    travel_id: None,
    waited_stdout: String::new(),
    commit: None,
});

const EFFECT_CLASSES: &[&str] = &[
    "clock",
    "need",
    "stock",
    "location",
    "relationship",
    "reputation",
    "memory",
    "rumor",
    "resource",
    "flag",
    "proposal",
    "message",
];

fn lock_session() -> std::sync::MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|err| err.into_inner())
}

fn lock_record() -> std::sync::MutexGuard<'static, Record> {
    RECORD.lock().unwrap_or_else(|err| err.into_inner())
}

fn clear_record() {
    *lock_record() = Record {
        wait_id: None,
        travel_id: None,
        waited_stdout: String::new(),
        commit: None,
    };
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
    if let Some(player) = session.player.as_mut() {
        stop_proc(player);
    }
    let _ = session.gm.shutdown(std::net::Shutdown::Both);
    stop_proc(&mut session.server);
    let pack = session.pack.clone();
    drop(session);
    let _ = fs::remove_dir_all(pack);
}

fn current_pack() -> PathBuf {
    lock_session()
        .as_ref()
        .expect("no reconnect shard")
        .pack
        .clone()
}

fn require_id(slot: &Option<String>, name: &str) -> String {
    slot.clone().unwrap_or_else(|| panic!("missing {name}"))
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

fn gm_session() -> TcpStream {
    let mut stream = TcpStream::connect("127.0.0.1:9737").unwrap();
    stream.set_nodelay(true).unwrap();
    stream
        .write_all(b"gm\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"proof\",\"version\":\"0\"}}}\n")
        .unwrap();
    let init = read_lf(&mut stream, Duration::from_secs(5));
    assert!(init.contains("\"result\""), "{init}");
    stream
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .unwrap();
    stream
}

fn read_lf(stream: &mut TcpStream, timeout: Duration) -> String {
    stream.set_read_timeout(Some(timeout)).unwrap();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => panic!("eof {}", String::from_utf8_lossy(&buf)),
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(err) => panic!("read {err}; {}", String::from_utf8_lossy(&buf)),
        }
    }
    String::from_utf8(buf).unwrap()
}

fn call(stream: &mut TcpStream, id: u64, name: &str, arguments: &str) -> Vec<String> {
    write!(
        stream,
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":{name_json},\"arguments\":{arguments}}}}}\n",
        name_json = serde_json::to_string(name).unwrap()
    )
    .unwrap();
    stream.flush().unwrap();
    let mut lines = Vec::new();
    let start = Instant::now();
    let marker = format!("\"id\":{id}");
    loop {
        if start.elapsed() > Duration::from_secs(5) {
            panic!("{name} stalled: {lines:?}");
        }
        lines.push(read_lf(stream, Duration::from_secs(5)));
        if lines.iter().any(|line: &String| line.contains(&marker)) {
            lines.extend(drain_ready(stream));
            break;
        }
    }
    lines
}

fn drain_ready(stream: &mut TcpStream) -> Vec<String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut lines = Vec::new();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    lines.push(String::from_utf8(buf.clone()).unwrap());
                    buf.clear();
                } else {
                    buf.push(byte[0]);
                }
            }
            Err(_) => break,
        }
    }
    lines
}

fn structured(line: &str) -> Value {
    let value: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}: {line}"));
    value
        .pointer("/result/structuredContent")
        .cloned()
        .unwrap_or_else(|| panic!("no structured content in {line}"))
}

fn result_line<'a>(lines: &'a [String], id: u64) -> &'a str {
    lines
        .iter()
        .find(|line| line.contains(&format!("\"id\":{id}")))
        .unwrap_or_else(|| panic!("no result {id} in {lines:?}"))
}

fn tool_body(stream: &mut TcpStream, id: u64, name: &str, arguments: &str) -> Value {
    let lines = call(stream, id, name, arguments);
    let line = result_line(&lines, id);
    assert!(line.contains("\"result\""), "{name}: {line}");
    structured(line)
}

fn gm_tool(name: &str, arguments: &str) -> Value {
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no reconnect shard");
    let id = session.next_id;
    let body = tool_body(&mut session.gm, id, name, arguments);
    session.next_id += 1;
    body
}

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

fn queued_ids(stdout: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("kind").and_then(|kind| kind.as_str()) == Some("queued") {
            ids.push(value["id"].as_str().unwrap().to_string());
        }
    }
    ids
}

fn queued_rows(status: &Value) -> Vec<(String, String)> {
    status["queued"]
        .as_array()
        .unwrap_or_else(|| panic!("no queued in {status}"))
        .iter()
        .map(|item| {
            (
                item["id"].as_str().unwrap().to_string(),
                item["move"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn assert_closed_effects(tick: &Value, travel_id: &str) {
    let rejected: Vec<_> = tick["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["id"] == travel_id)
        .collect();
    assert_eq!(rejected.len(), 1, "{tick}");
    assert!(
        tick["intents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|intent| intent["id"] != travel_id),
        "{tick}"
    );
    assert!(
        tick["effects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|effect| effect["intent_id"] != travel_id),
        "{tick}"
    );
    for effect in tick["effects"].as_array().unwrap() {
        let class = effect["class"].as_str().unwrap();
        assert!(EFFECT_CLASSES.contains(&class), "{effect}");
        if class == "location" {
            assert_ne!(effect["actor_id"], "pc_rowan", "{effect}");
        }
    }
}

fn stored_queue() -> Vec<(String, String)> {
    let record = lock_record();
    let wait_id = require_id(&record.wait_id, "wait id");
    let travel_id = require_id(&record.travel_id, "travel id");
    vec![(wait_id, "wait".to_string()), (travel_id, "travel".to_string())]
}

fn listen(w: &mut World, label: &str) {
    drop_session();
    let pack = support::temp_pack(label);
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let gm = gm_session();
    let player = seat_player(&pack);
    w.pack = Some(pack.clone());
    *lock_session() = Some(Session {
        player: Some(player),
        gm,
        server,
        next_id: 2,
        pack,
    });
}

fn wait_humans(pack: &Path, humans: u32, timeout: Duration) -> spookery_kernel::model::Now {
    let start = Instant::now();
    loop {
        if let Ok(shard) = spookery_kernel::open_shard(pack) {
            if shard.now().presence.humans == humans {
                return shard.now().clone();
            }
        }
        if start.elapsed() > timeout {
            panic!("humans did not become {humans}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[given(expr = "the reconnect port is locked")]
fn reconnect_port_locked(w: &mut World) {
    clear_record();
    drop_session();
    w.pack = None;
    w.port = Some(crate::PortGuard(support::lock_port()));
}

#[when(expr = "a reconnect player sits with a GM on shard {string}")]
fn player_sits(w: &mut World, label: String) {
    listen(w, &label);
}

#[then(expr = "reconnect humans is {int}")]
fn reconnect_humans(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().presence.humans as i64, n);
}

#[then(expr = "reconnect humans becomes {int}")]
fn reconnect_humans_becomes(w: &mut World, n: i64) {
    let _ = w;
    let dropped = wait_humans(&current_pack(), n as u32, Duration::from_secs(5));
    assert_eq!(dropped.presence.humans as i64, n);
}

#[then(expr = "reconnect actors present are {word}")]
fn actors_present(w: &mut World, who: String) {
    let _ = w;
    let present = current_now().presence.actors_present;
    if who == "empty" {
        assert!(present.is_empty(), "{present:?}");
    } else {
        assert_eq!(present, vec![who]);
    }
}

#[then(expr = "reconnect world_time is {int}")]
fn world_time_is(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().world_time, n as u64);
}

#[then(expr = "reconnect world_time is not {int}")]
fn world_time_is_not(w: &mut World, n: i64) {
    let _ = w;
    assert_ne!(current_now().world_time, n as u64);
}

#[then(expr = "reconnect tick_index is {int}")]
fn tick_index_is(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().tick_index, n as u64);
}

#[then(expr = "reconnect Rowan is in {word}")]
fn rowan_is_in(w: &mut World, place: String) {
    let _ = w;
    assert_eq!(rowan_place(&current_now()), place);
}

#[then(expr = "reconnect tick files are none")]
fn tick_files_none(w: &mut World) {
    let _ = w;
    assert!(tick_names(&current_pack()).is_empty());
}

#[then(expr = "reconnect tick files are {string}")]
fn tick_files_are(w: &mut World, list: String) {
    let _ = w;
    let names: Vec<String> = list.split(',').map(str::to_string).collect();
    assert_eq!(tick_names(&current_pack()), names);
}

#[when(expr = "the reconnect player sends {string}")]
fn player_sends(w: &mut World, line: String) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no reconnect shard");
    let player = session.player.as_mut().expect("no reconnect player");
    write_stdin(player, &line);
}

#[then(expr = "the reconnect player stdout contains {string}")]
fn stdout_contains(w: &mut World, needle: String) {
    let _ = w;
    let guard = lock_session();
    let session = guard.as_ref().expect("no reconnect shard");
    let player = session.player.as_ref().expect("no reconnect player");
    wait_has(&player.stdout, &needle, Duration::from_secs(5));
}

#[then(expr = "reconnect queued lines are {int}")]
fn queued_lines(w: &mut World, n: i64) {
    let _ = w;
    let stdout = {
        let guard = lock_session();
        let session = guard.as_ref().expect("no reconnect shard");
        let player = session.player.as_ref().expect("no reconnect player");
        wait_count(
            &player.stdout,
            "\"kind\":\"queued\"",
            n as usize,
            Duration::from_secs(5),
        )
    };
    let ids = queued_ids(&stdout);
    assert_eq!(ids.len(), n as usize, "{stdout}");
    let mut record = lock_record();
    if n == 1 {
        record.waited_stdout = stdout;
        record.wait_id = Some(ids[0].clone());
    } else {
        assert_eq!(queued_ids(&record.waited_stdout), vec![ids[0].clone()], "{stdout}");
        assert_ne!(ids[0], ids[1]);
        record.wait_id = Some(ids[0].clone());
        record.travel_id = Some(ids[1].clone());
    }
}

#[then(expr = "the reconnect GM status shows humans {int} and the stored queue")]
fn gm_status_queue(w: &mut World, humans: i64) {
    let _ = w;
    let expect = stored_queue();
    let status = gm_tool("session_status", "{}");
    assert_eq!(
        status["presence"]["humans"].as_i64(),
        Some(humans),
        "{status}"
    );
    assert_eq!(queued_rows(&status), expect, "{status}");
}

#[then(expr = "reconnect now.yaml bytes contain neither queued id")]
fn now_omits_ids(w: &mut World) {
    let _ = w;
    let record = lock_record();
    let wait_id = require_id(&record.wait_id, "wait id");
    let travel_id = require_id(&record.travel_id, "travel id");
    drop(record);
    let stored = fs::read(current_pack().join("state/now.yaml")).unwrap();
    assert!(!stored
        .windows(wait_id.len())
        .any(|window| window == wait_id.as_bytes()));
    assert!(!stored
        .windows(travel_id.len())
        .any(|window| window == travel_id.as_bytes()));
}

#[then(expr = "the reconnect player exits 0")]
fn player_exits_0(w: &mut World) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no reconnect shard");
    let player = session.player.as_mut().expect("no reconnect player");
    assert_eq!(
        wait_exit(player, Duration::from_secs(5)).code(),
        Some(0)
    );
}

#[when(expr = "the reconnect player sits again")]
fn player_sits_again(w: &mut World) {
    let _ = w;
    let pack = current_pack();
    let player = seat_player(&pack);
    lock_session()
        .as_mut()
        .expect("no reconnect shard")
        .player = Some(player);
}

#[then(expr = "reconnect scrollback has one say of {string}")]
fn scrollback_say(w: &mut World, text: String) {
    let _ = w;
    let back = {
        let guard = lock_session();
        let session = guard.as_ref().expect("no reconnect shard");
        let player = session.player.as_ref().expect("no reconnect player");
        text_of(&player.stdout)
    };
    let say_lines = back
        .lines()
        .filter(|line| line.contains("\"kind\":\"say\"") && line.contains(&text))
        .count();
    assert_eq!(say_lines, 1, "{back}");
}

#[then(expr = "reconnect session tape contains {string} once")]
fn session_tape_once(w: &mut World, text: String) {
    let _ = w;
    let tape = fs::read_to_string(current_pack().join("state/log/session.jsonl")).unwrap();
    assert_eq!(tape.matches(text.as_str()).count(), 1, "{tape}");
}

#[when(expr = "the reconnect GM commits the turn including a tick")]
fn gm_commits(w: &mut World) {
    let _ = w;
    let body = gm_tool("commit_turn", r#"{"include_tick":true}"#);
    lock_record().commit = Some(body);
}

#[when(expr = "the reconnect GM commits the turn including a tick again")]
fn gm_commits_again(w: &mut World) {
    gm_commits(w);
}

#[then(expr = "the reconnect commit has no code and log_path ends with {string}")]
fn commit_log_path(w: &mut World, suffix: String) {
    let _ = w;
    let committed = lock_record().commit.clone().expect("no reconnect commit");
    assert!(committed.get("code").is_none(), "{committed}");
    assert!(
        committed["log_path"].as_str().unwrap().ends_with(&suffix),
        "{committed}"
    );
}

#[then(expr = "the reconnect commit has no code and log_path is null")]
fn commit_log_path_null(w: &mut World) {
    let _ = w;
    let committed = lock_record().commit.clone().expect("no reconnect commit");
    assert!(committed.get("code").is_none(), "{committed}");
    assert!(committed["log_path"].is_null(), "{committed}");
}

#[then(expr = "reconnect tick {string} records humans {int} from {int} to {int}")]
fn tick_records_humans(w: &mut World, name: String, humans: i64, before: i64, after: i64) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    assert_eq!(tick["presence_humans"].as_i64(), Some(humans), "{tick}");
    assert_eq!(tick["world_time_before"].as_i64(), Some(before), "{tick}");
    assert_eq!(tick["world_time_after"].as_i64(), Some(after), "{tick}");
}

#[then(expr = "reconnect tick {string} applies the wait id once")]
fn tick_applies_wait(w: &mut World, name: String) {
    let _ = w;
    let wait_id = require_id(&lock_record().wait_id, "wait id");
    let tick = tick_value(&current_pack(), &name);
    let waits: Vec<_> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["id"] == wait_id)
        .collect();
    assert_eq!(waits.len(), 1, "{tick}");
    assert_eq!(waits[0]["move"], "wait");
}

#[then(expr = "reconnect tick {string} rejects the travel id as {word}")]
fn tick_rejects_travel(w: &mut World, name: String, reason: String) {
    let _ = w;
    let travel_id = require_id(&lock_record().travel_id, "travel id");
    let tick = tick_value(&current_pack(), &name);
    assert_eq!(
        tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["id"] == travel_id)
            .count(),
        1,
        "{tick}"
    );
    assert_eq!(
        tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == travel_id)
            .unwrap()["reason"],
        reason,
        "{tick}"
    );
}

#[then(expr = "reconnect tick {string} keeps the travel id out of intents")]
fn tick_omits_travel_intent(w: &mut World, name: String) {
    let _ = w;
    let travel_id = require_id(&lock_record().travel_id, "travel id");
    let tick = tick_value(&current_pack(), &name);
    assert!(tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|intent| intent["id"] != travel_id));
}

#[then(expr = "reconnect tick {string} closes effects on the travel id")]
fn tick_closes_effects(w: &mut World, name: String) {
    let _ = w;
    let travel_id = require_id(&lock_record().travel_id, "travel id");
    assert_closed_effects(&tick_value(&current_pack(), &name), &travel_id);
}

#[then(expr = "the reconnect GM queue is empty")]
fn gm_queue_empty(w: &mut World) {
    let _ = w;
    let status = gm_tool("session_status", "{}");
    assert!(queued_rows(&status).is_empty(), "{status}");
    let finished = lock_record()
        .commit
        .as_ref()
        .is_some_and(|commit| commit["log_path"].is_null());
    if finished {
        drop_session();
    }
}
