use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, text_of, wait_count, wait_has, write_stdin, Proc};
use crate::World;

// Procs and the GM socket cannot live on World. One scenario runs at a time.
struct Session {
    server: Proc,
    player: Proc,
    gm: Option<TcpStream>,
    next_id: u64,
    schema_seen: usize,
    spec_seen: usize,
    stdout_before: String,
    pack: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
struct Derived {
    kernel: Vec<Value>,
    rejected: Vec<(String, String)>,
    npc_places: Vec<(String, String)>,
}

struct Record {
    baseline: Option<Derived>,
    seated: Option<Derived>,
    live: Option<Derived>,
    opened_last_tick_at: Option<String>,
    live_last_tick_at: Option<String>,
    travel_id: Option<String>,
    missing_id: Option<String>,
    live_travel_id: Option<String>,
    commit: Option<Value>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static RECORD: Mutex<Record> = Mutex::new(Record {
    baseline: None,
    seated: None,
    live: None,
    opened_last_tick_at: None,
    live_last_tick_at: None,
    travel_id: None,
    missing_id: None,
    live_travel_id: None,
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
        baseline: None,
        seated: None,
        live: None,
        opened_last_tick_at: None,
        live_last_tick_at: None,
        travel_id: None,
        missing_id: None,
        live_travel_id: None,
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
    stop_proc(&mut session.player);
    session.gm.take();
    stop_proc(&mut session.server);
    let pack = session.pack.clone();
    drop(session);
    let _ = fs::remove_dir_all(pack);
}

fn current_pack() -> PathBuf {
    lock_session()
        .as_ref()
        .expect("no travel shard")
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

fn npc_places(now: &spookery_kernel::model::Now) -> Vec<(String, String)> {
    let mut rows: Vec<_> = now
        .actors
        .iter()
        .filter(|(_, actor)| !actor.is_pc())
        .map(|(id, actor)| (id.clone(), actor.place().to_string()))
        .collect();
    rows.sort();
    rows
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

fn load_tick(name: &str) -> Value {
    tick_value(&current_pack(), name)
}

fn kernel_intents(tick: &Value) -> Vec<Value> {
    let mut rows: Vec<Value> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["source"] == "kernel")
        .cloned()
        .collect();
    rows.sort_by(|left, right| {
        left["id"]
            .as_str()
            .unwrap()
            .cmp(right["id"].as_str().unwrap())
    });
    rows
}

fn rejected_except(tick: &Value, skip: &[&str]) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = tick["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| skip.iter().all(|id| item["id"] != *id))
        .map(|item| {
            (
                item["id"].as_str().unwrap().to_string(),
                item["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    rows.sort();
    rows
}

fn derived(tick: &Value, now: &spookery_kernel::model::Now, skip: &[&str]) -> Derived {
    Derived {
        kernel: kernel_intents(tick),
        rejected: rejected_except(tick, skip),
        npc_places: npc_places(now),
    }
}

fn derived_now(tick_name: &str, skip: &[&str]) -> Derived {
    let pack = current_pack();
    let tick = tick_value(&pack, tick_name);
    let now = now_of(&pack);
    derived(&tick, &now, skip)
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
            assert_ne!(effect["payload"]["actor_id"], "pc_rowan", "{effect}");
        }
    }
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
    let session = guard.as_mut().expect("no travel shard");
    let id = session.next_id;
    let stream = session.gm.as_mut().expect("no GM");
    let body = tool_body(stream, id, name, arguments);
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

fn listen(w: &mut World, label: &str, with_gm: bool) {
    drop_session();
    let pack = support::temp_pack(label);
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let gm = if with_gm { Some(gm_session()) } else { None };
    let player = seat_player(&pack);
    w.pack = Some(pack.clone());
    *lock_session() = Some(Session {
        server,
        player,
        gm,
        next_id: 2,
        schema_seen: 0,
        spec_seen: 0,
        stdout_before: String::new(),
        pack,
    });
}

fn expected_ticks(list: &str) -> Vec<String> {
    list.split(',').map(str::to_string).collect()
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

#[given(expr = "the travel port is locked")]
fn travel_port_locked(w: &mut World) {
    clear_record();
    drop_session();
    w.pack = None;
    w.port = Some(crate::PortGuard(support::lock_port()));
}

#[when(expr = "a travel player sits on shard {string}")]
fn player_sits(w: &mut World, label: String) {
    listen(w, &label, false);
}

#[when(expr = "a travel player sits with a GM on shard {string}")]
fn player_sits_with_gm(w: &mut World, label: String) {
    listen(w, &label, true);
}

#[then(expr = "travel humans is {int}")]
fn travel_humans(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().presence.humans as i64, n);
}

#[then(expr = "travel world_time is {int}")]
fn travel_world_time(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().world_time, n as u64);
}

#[then(expr = "travel world_time is not {int}")]
fn travel_world_time_is_not(w: &mut World, n: i64) {
    let _ = w;
    assert_ne!(current_now().world_time, n as u64);
}

#[then(expr = "travel tick_index is {int}")]
fn travel_tick_index(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().tick_index, n as u64);
}

#[then(expr = "travel Rowan is in {word}")]
fn travel_rowan(w: &mut World, place: String) {
    let _ = w;
    assert_eq!(rowan_place(&current_now()), place);
}

#[then(expr = "travel tick files are none")]
fn travel_ticks_none(w: &mut World) {
    let _ = w;
    assert!(tick_names(&current_pack()).is_empty());
}

#[then(expr = "travel tick files are {string}")]
fn travel_ticks_are(w: &mut World, list: String) {
    let _ = w;
    assert_eq!(tick_names(&current_pack()), expected_ticks(&list));
}

#[when(expr = "the travel player sends {string}")]
fn player_sends(w: &mut World, line: String) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no travel shard");
    session.stdout_before = text_of(&session.player.stdout);
    write_stdin(&mut session.player, &line);
}

#[then(expr = "the travel player stdout gains one resume")]
fn gains_one_resume(w: &mut World) {
    let _ = w;
    let guard = lock_session();
    let session = guard.as_ref().expect("no travel shard");
    let want = session.stdout_before.matches("\"kind\":\"resume\"").count() + 1;
    wait_count(
        &session.player.stdout,
        "\"kind\":\"resume\"",
        want,
        Duration::from_secs(5),
    );
}

#[then(expr = "travel tick {string} includes a player wait intent")]
fn player_wait_intent(w: &mut World, name: String) {
    let _ = w;
    let tick = load_tick(&name);
    assert!(tick["intents"].as_array().unwrap().iter().any(|intent| {
        intent["move"] == "wait" && intent["source"] == "player"
    }));
}

#[then(expr = "that tick is the travel derivation baseline")]
fn save_baseline(w: &mut World) {
    let _ = w;
    lock_record().baseline = Some(derived_now("tick-0001.jsonl", &[]));
}

#[then(expr = "the travel opened clock is stored")]
fn store_opened_clock(w: &mut World) {
    let _ = w;
    lock_record().opened_last_tick_at = Some(current_now().last_tick_at);
}

#[then(expr = "the travel live clock is stored")]
fn store_live_clock(w: &mut World) {
    let _ = w;
    lock_record().live_last_tick_at = Some(current_now().last_tick_at);
}

#[when(expr = "the travel player line {string} is refused {word}")]
fn player_refused(w: &mut World, line: String, code: String) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no travel shard");
    let stdout = text_of(&session.player.stdout);
    write_stdin(&mut session.player, &line);
    let seen = if code == "SCHEMA" {
        session.schema_seen += 1;
        session.schema_seen
    } else if code == "NOT_IN_SPEC" {
        session.spec_seen += 1;
        session.spec_seen
    } else {
        panic!("unexpected refusal code {code}");
    };
    let stderr = wait_count(&session.player.stderr, &code, seen, Duration::from_secs(5));
    assert_eq!(stderr.matches(&code).count(), seen, "{line}\n{stderr}");
    assert_eq!(text_of(&session.player.stdout), stdout, "{line} wrote on stdout");
}

#[then(expr = "the travel GM queue is empty")]
fn gm_queue_empty(w: &mut World) {
    let _ = w;
    let status = gm_tool("session_status", "{}");
    assert!(queued_rows(&status).is_empty(), "{status}");
}

#[then(expr = "the travel player stdout gains one queued travel id and no resume or refusal")]
fn gains_queued_travel(w: &mut World) {
    let _ = w;
    let guard = lock_session();
    let session = guard.as_ref().expect("no travel shard");
    let stdout = wait_has(
        &session.player.stdout,
        "\"kind\":\"queued\"",
        Duration::from_secs(5),
    );
    let added = &stdout[session.stdout_before.len()..];
    assert!(added.contains("\"kind\":\"queued\""), "{added}");
    assert!(!added.contains("\"kind\":\"resume\""), "{added}");
    assert!(!added.contains("\"kind\":\"refused\""), "{added}");
    let ids = queued_ids(added);
    assert_eq!(ids.len(), 1, "{added}");
    drop(guard);
    lock_record().travel_id = Some(ids[0].clone());
}

#[then(expr = "the travel GM queue is the travel id moving travel")]
fn gm_queue_travel(w: &mut World) {
    let _ = w;
    let id = require_id(&lock_record().travel_id, "travel id");
    let status = gm_tool("session_status", "{}");
    assert_eq!(
        queued_rows(&status),
        vec![(id, "travel".to_string())],
        "{status}"
    );
}

#[then(expr = "the travel now.yaml bytes do not contain the travel id")]
fn now_omits_travel_id(w: &mut World) {
    let _ = w;
    let id = require_id(&lock_record().travel_id, "travel id");
    let bytes = fs::read(current_pack().join("state/now.yaml")).unwrap();
    assert!(!bytes
        .windows(id.len())
        .any(|window| window == id.as_bytes()));
}

#[when(expr = "the travel GM commits the turn including a tick")]
fn gm_commits(w: &mut World) {
    let _ = w;
    let body = gm_tool("commit_turn", r#"{"include_tick":true}"#);
    lock_record().commit = Some(body);
}

#[then(expr = "the travel commit has no code and log_path ends with {string}")]
fn commit_log_path(w: &mut World, suffix: String) {
    let _ = w;
    let committed = lock_record()
        .commit
        .clone()
        .expect("no travel commit");
    assert!(committed.get("code").is_none(), "{committed}");
    assert!(
        committed["log_path"].as_str().unwrap().ends_with(&suffix),
        "{committed}"
    );
}

#[then(expr = "the travel second commit has no code")]
fn second_commit_has_no_code(w: &mut World) {
    let _ = w;
    let committed = lock_record()
        .commit
        .clone()
        .expect("no travel commit");
    assert!(committed.get("code").is_none(), "{committed}");
}

#[then(expr = "travel last_tick_at differs from the opened clock")]
fn opened_clock_changed(w: &mut World) {
    let _ = w;
    let opened = lock_record()
        .opened_last_tick_at
        .clone()
        .expect("no opened clock");
    assert_ne!(current_now().last_tick_at, opened);
}

#[then(expr = "travel last_tick_at differs from the live clock")]
fn live_clock_changed(w: &mut World) {
    let _ = w;
    let opened = lock_record()
        .live_last_tick_at
        .clone()
        .expect("no live clock");
    assert_ne!(current_now().last_tick_at, opened);
}

#[then(expr = "travel tick {string} records humans {int} from {int} to {int}")]
fn tick_records_humans(w: &mut World, name: String, humans: i64, before: i64, after: i64) {
    let _ = w;
    let tick = load_tick(&name);
    assert_eq!(tick["presence_humans"].as_i64(), Some(humans), "{tick}");
    assert_eq!(tick["world_time_before"].as_i64(), Some(before), "{tick}");
    assert_eq!(tick["world_time_after"].as_i64(), Some(after), "{tick}");
}

#[then(expr = "travel tick {string} records time from {int} to {int}")]
fn tick_records_time(w: &mut World, name: String, before: i64, after: i64) {
    let _ = w;
    let tick = load_tick(&name);
    assert_eq!(tick["world_time_before"].as_i64(), Some(before), "{tick}");
    assert_eq!(tick["world_time_after"].as_i64(), Some(after), "{tick}");
}

#[then(expr = "travel tick {string} rejects the travel id as {word}")]
fn rejects_travel_id(w: &mut World, name: String, reason: String) {
    let _ = w;
    let id = require_id(&lock_record().travel_id, "travel id");
    let tick = load_tick(&name);
    assert_eq!(
        tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == id)
            .unwrap()["reason"]
            .as_str(),
        Some(reason.as_str()),
        "{tick}"
    );
}

#[then(expr = "travel tick {string} rejects the missing id as {word}")]
fn rejects_missing_id(w: &mut World, name: String, reason: String) {
    let _ = w;
    let id = require_id(&lock_record().missing_id, "missing id");
    let tick = load_tick(&name);
    assert_eq!(
        tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == id)
            .unwrap()["reason"]
            .as_str(),
        Some(reason.as_str()),
        "{tick}"
    );
}

#[then(expr = "travel tick {string} closes effects on the travel id")]
fn closes_travel_id(w: &mut World, name: String) {
    let _ = w;
    let id = require_id(&lock_record().travel_id, "travel id");
    assert_closed_effects(&load_tick(&name), &id);
}

#[then(expr = "travel tick {string} closes effects on the missing id")]
fn closes_missing_id(w: &mut World, name: String) {
    let _ = w;
    let id = require_id(&lock_record().missing_id, "missing id");
    assert_closed_effects(&load_tick(&name), &id);
}

#[then(expr = "travel tick {string} closes effects on the live travel id")]
fn closes_live_id(w: &mut World, name: String) {
    let _ = w;
    let id = require_id(&lock_record().live_travel_id, "live travel id");
    assert_closed_effects(&load_tick(&name), &id);
}

#[then(expr = "that tick is the travel seated derivation skipping the travel id")]
fn save_seated(w: &mut World) {
    let _ = w;
    let id = require_id(&lock_record().travel_id, "travel id");
    lock_record().seated = Some(derived_now("tick-0001.jsonl", &[id.as_str()]));
}

#[then(expr = "the travel player stdout gains one new queued id stored as the missing id")]
fn gains_missing_id(w: &mut World) {
    let _ = w;
    let guard = lock_session();
    let session = guard.as_ref().expect("no travel shard");
    let stdout = wait_count(
        &session.player.stdout,
        "\"kind\":\"queued\"",
        2,
        Duration::from_secs(5),
    );
    let added = &stdout[session.stdout_before.len()..];
    let ids = queued_ids(added);
    assert_eq!(ids.len(), 1, "{added}");
    let missing_id = ids[0].clone();
    drop(guard);
    let travel_id = require_id(&lock_record().travel_id, "travel id");
    assert_ne!(missing_id, travel_id);
    lock_record().missing_id = Some(missing_id);
}

#[then(expr = "the travel GM queue is the missing id moving travel")]
fn gm_queue_missing(w: &mut World) {
    let _ = w;
    let id = require_id(&lock_record().missing_id, "missing id");
    let status = gm_tool("session_status", "{}");
    assert_eq!(
        queued_rows(&status),
        vec![(id, "travel".to_string())],
        "{status}"
    );
}

#[then(expr = "travel tick {string} leaves the travel id out of rejected and intents")]
fn omits_travel_id(w: &mut World, name: String) {
    let _ = w;
    let id = require_id(&lock_record().travel_id, "travel id");
    let tick = load_tick(&name);
    assert!(tick["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["id"] != id));
    assert!(tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|intent| intent["id"] != id));
}

#[then(expr = "the travel player stdout gains one resume and no queued line")]
fn gains_live_resume(w: &mut World) {
    let _ = w;
    let guard = lock_session();
    let session = guard.as_ref().expect("no travel shard");
    let want = session.stdout_before.matches("\"kind\":\"resume\"").count() + 1;
    let stdout = wait_count(
        &session.player.stdout,
        "\"kind\":\"resume\"",
        want,
        Duration::from_secs(5),
    );
    let added = &stdout[session.stdout_before.len()..];
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
    assert!(!added.contains("\"kind\":\"queued\""), "{added}");
}

#[then(expr = "travel tick {string} has one new rejection {word} beyond the baseline")]
fn one_new_rejection(w: &mut World, name: String, reason: String) {
    let _ = w;
    let tick = load_tick(&name);
    let baseline = lock_record()
        .baseline
        .clone()
        .expect("no travel baseline")
        .rejected;
    let extra: Vec<_> = rejected_except(&tick, &[])
        .into_iter()
        .filter(|row| !baseline.iter().any(|kept| kept == row))
        .collect();
    assert_eq!(extra.len(), 1, "{tick}");
    assert_eq!(extra[0].1, reason);
    lock_record().live_travel_id = Some(extra[0].0.clone());
}

#[then(expr = "that tick is the travel live derivation skipping the live travel id")]
fn save_live(w: &mut World) {
    let _ = w;
    let id = require_id(&lock_record().live_travel_id, "live travel id");
    lock_record().live = Some(derived_now("tick-0001.jsonl", &[id.as_str()]));
}

fn assert_same(left: &Derived, right: &Derived) {
    assert_eq!(left.kernel, right.kernel);
    assert_eq!(left.rejected, right.rejected);
    assert_eq!(left.npc_places, right.npc_places);
}

#[then(expr = "the travel seated derivation matches the baseline")]
fn seated_matches_baseline(w: &mut World) {
    let _ = w;
    let record = lock_record();
    let baseline = record.baseline.as_ref().expect("no travel baseline");
    let seated = record.seated.as_ref().expect("no seated derivation");
    assert_same(seated, baseline);
}

#[then(expr = "the travel live derivation matches the baseline")]
fn live_matches_baseline(w: &mut World) {
    let _ = w;
    let record = lock_record();
    let baseline = record.baseline.as_ref().expect("no travel baseline");
    let live = record.live.as_ref().expect("no live derivation");
    assert_same(live, baseline);
}

#[then(expr = "the travel shard stops")]
fn travel_stops(w: &mut World) {
    drop_session();
    w.pack = None;
}
