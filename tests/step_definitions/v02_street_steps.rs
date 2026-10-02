use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
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
    stdout_mark: String,
    added: String,
    pack: PathBuf,
    last_queued_id: String,
    world_time_mark: u64,
    last_tool: Value,
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
    session.gm.take();
    stop_proc(&mut session.server);
    let pack = session.pack.clone();
    drop(session);
    let _ = fs::remove_dir_all(pack);
}

fn current_pack() -> PathBuf {
    lock_session()
        .as_ref()
        .expect("no street session")
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

fn tick_value(name: &str) -> Value {
    let text = fs::read_to_string(current_pack().join("state/log").join(name)).unwrap();
    serde_json::from_str(text.trim()).unwrap()
}

fn last_perceive(stdout: &str) -> Value {
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.contains("\"kind\":\"perceive\""))
        .unwrap_or_else(|| panic!("no perceive line in {stdout}"));
    serde_json::from_str(line).unwrap_or_else(|err| panic!("perceive: {err} in {line}"))
}

fn assert_street(stdout: &str, now: &spookery_kernel::model::Now) {
    let view = last_perceive(stdout);
    assert_eq!(view["kind"], "perceive");
    assert_eq!(view["place"], "docks_street");
    assert_eq!(rowan_place(now), "docks_street");
    let mut visible: Vec<String> = view["visible_actors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    visible.sort();
    let mut there: Vec<String> = now
        .actors
        .iter()
        .filter(|(_, actor)| actor.place() == "docks_street")
        .map(|(id, _)| id.clone())
        .collect();
    there.sort();
    assert!(visible.iter().any(|id| id == "pc_rowan"), "{view}");
    assert_eq!(visible, there, "{view}");
}

fn pc_location_count(tick: &Value) -> usize {
    tick["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| {
            effect["class"] == "location" && effect["payload"]["actor_id"] == "pc_rowan"
        })
        .count()
}

fn assert_street_effect(tick: &Value, intent_id: &str) {
    let hits: Vec<_> = tick["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| {
            effect["class"] == "location" && effect["payload"]["actor_id"] == "pc_rowan"
        })
        .collect();
    assert_eq!(hits.len(), 1, "{tick}");
    assert_eq!(hits[0]["intent_id"], intent_id);
    assert_eq!(hits[0]["as"], "player:pc_rowan");
    assert_eq!(hits[0]["payload"]["from"], "docks_shop");
    assert_eq!(hits[0]["payload"]["to"], "docks_street");
}

fn assert_no_pc_kernel(tick: &Value) {
    let tick_index = tick["tick_index"].as_u64().unwrap();
    for move_name in ["travel", "work", "rest", "restock", "wait"] {
        let id = spookery_kernel::derive::kernel_intent_id(
            "docks-001",
            tick_index,
            "pc_rowan",
            move_name,
        );
        assert!(
            tick["intents"]
                .as_array()
                .unwrap()
                .iter()
                .all(|intent| intent["id"] != id),
            "derived {move_name}"
        );
        assert!(
            tick["rejected"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["id"] != id),
            "rejected derived {move_name}"
        );
    }
    assert!(tick["intents"].as_array().unwrap().iter().all(|intent| {
        intent["actor_id"] != "pc_rowan" || intent["source"] != "kernel"
    }));
}

fn player_rejections(tick: &Value) -> Vec<&Value> {
    tick["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["id"].as_str().is_some_and(|id| id.contains('-')))
        .collect()
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

fn player_stdout() -> (Arc<Mutex<String>>, String) {
    let guard = lock_session();
    let session = guard.as_ref().expect("no street session");
    (session.player.stdout.clone(), session.stdout_mark.clone())
}

fn store_added(added: String) {
    lock_session().as_mut().expect("no street session").added = added;
}

fn remember_one_queued(added: &str) {
    let ids = queued_ids(added);
    assert_eq!(ids.len(), 1, "{added}");
    lock_session()
        .as_mut()
        .expect("no street session")
        .last_queued_id = ids[0].clone();
}

fn slice_added(full: &str, mark: &str) -> String {
    assert!(
        full.starts_with(mark),
        "stdout prefix changed\nmark:\n{mark}\nfull:\n{full}"
    );
    full[mark.len()..].to_string()
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

fn gm_call(name: &str, arguments: &str) -> Value {
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no street session");
    let id = session.next_id;
    let body = {
        let stream = session.gm.as_mut().expect("no GM");
        tool_body(stream, id, name, arguments)
    };
    session.next_id = id + 1;
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
    let pack_arg = support::pack_arg(&pack);
    let mut server = support::spawn(&["run", &pack_arg]);
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
        stdout_mark: String::new(),
        added: String::new(),
        pack,
        last_queued_id: String::new(),
        world_time_mark: 0,
        last_tool: Value::Null,
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

fn player_text() -> String {
    let stdout = lock_session()
        .as_ref()
        .expect("no street session")
        .player
        .stdout
        .clone();
    text_of(&stdout)
}

#[given(expr = "the street port is locked")]
fn street_port_locked(w: &mut World) {
    drop_session();
    w.pack = None;
    w.port = Some(crate::PortGuard(support::lock_port()));
}

#[when(expr = "a street player sits with a GM on shard {string}")]
fn street_sits_with_gm(w: &mut World, label: String) {
    listen(w, &label, true);
}

#[when(expr = "a street player sits on shard {string}")]
fn street_sits(w: &mut World, label: String) {
    listen(w, &label, false);
}

#[when(expr = "the street player enters {string}")]
fn street_enters(w: &mut World, line: String) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no street session");
    session.stdout_mark = text_of(&session.player.stdout);
    write_stdin(&mut session.player, &line);
}

#[when(expr = "the street player stdout is marked")]
fn street_mark_stdout(w: &mut World) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no street session");
    session.stdout_mark = text_of(&session.player.stdout);
}

#[when(expr = "the street world_time is marked")]
fn street_mark_time(w: &mut World) {
    let _ = w;
    let time = current_now().world_time;
    lock_session()
        .as_mut()
        .expect("no street session")
        .world_time_mark = time;
}

#[when(expr = "the street GM commits the turn")]
fn street_commit(w: &mut World) {
    let _ = w;
    let body = gm_call("commit_turn", r#"{"include_tick":true}"#);
    lock_session().as_mut().expect("no street session").last_tool = body;
}

#[then(expr = "street humans is {int}")]
fn street_humans(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().presence.humans as i64, n);
}

#[then(expr = "street world_time is {int}")]
fn street_world_time(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().world_time, n as u64);
}

#[then(expr = "street tick_index is {int}")]
fn street_tick_index(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().tick_index, n as u64);
}

#[then(expr = "street Rowan is in {word}")]
fn street_rowan(w: &mut World, place: String) {
    let _ = w;
    assert_eq!(rowan_place(&current_now()), place);
}

#[then(expr = "street has no tick log")]
fn street_no_ticks(w: &mut World) {
    let _ = w;
    assert!(tick_names(&current_pack()).is_empty());
}

#[then(expr = "street tick logs are {string}")]
fn street_tick_logs(w: &mut World, name: String) {
    let _ = w;
    assert_eq!(tick_names(&current_pack()), vec![name]);
}

#[then(expr = "the street perceive place is {string}")]
fn street_perceive_place(w: &mut World, place: String) {
    let _ = w;
    let view = last_perceive(&player_text());
    assert_eq!(view["place"].as_str(), Some(place.as_str()));
}

#[then(expr = "the street player stdout contains {string}")]
fn street_stdout_contains(w: &mut World, needle: String) {
    let _ = w;
    let (stdout, _) = player_stdout();
    let text = wait_has(&stdout, &needle, Duration::from_secs(5));
    assert!(text.contains(&needle), "{text}");
}

#[then(expr = "the street player queued count is {int} with no resume")]
fn street_queued(w: &mut World, count: i64) {
    let _ = w;
    let (stdout, mark) = player_stdout();
    let full = wait_count(
        &stdout,
        "\"kind\":\"queued\"",
        count as usize,
        Duration::from_secs(5),
    );
    let added = slice_added(&full, &mark);
    assert!(!added.contains("\"kind\":\"resume\""), "{added}");
    remember_one_queued(&added);
    store_added(added);
}

#[then(expr = "the street player queued count is {int} with no resume and no street place")]
fn street_queued_unmoved(w: &mut World, count: i64) {
    let _ = w;
    let (stdout, mark) = player_stdout();
    let full = wait_count(
        &stdout,
        "\"kind\":\"queued\"",
        count as usize,
        Duration::from_secs(5),
    );
    let added = slice_added(&full, &mark);
    assert!(!added.contains("\"kind\":\"resume\""), "{added}");
    assert!(!added.contains("\"place\":\"docks_street\""), "{added}");
    remember_one_queued(&added);
    store_added(added);
}

#[then(expr = "the street session queue is only the last queued id as {string}")]
fn street_session_queue(w: &mut World, move_name: String) {
    let _ = w;
    let id = lock_session()
        .as_ref()
        .expect("no street session")
        .last_queued_id
        .clone();
    let status = gm_call("session_status", "{}");
    assert_eq!(queued_rows(&status), vec![(id, move_name)]);
}

#[then(expr = "the street commit has no code")]
fn street_commit_ok(w: &mut World) {
    let _ = w;
    let body = lock_session()
        .as_ref()
        .expect("no street session")
        .last_tool
        .clone();
    assert!(body.get("code").is_none(), "{body}");
}

#[then(expr = "street tick {string} records the last queued id as player move {string}")]
fn street_player_move(w: &mut World, name: String, move_name: String) {
    let _ = w;
    let id = lock_session()
        .as_ref()
        .expect("no street session")
        .last_queued_id
        .clone();
    let tick = tick_value(&name);
    assert!(
        tick["intents"].as_array().unwrap().iter().any(|intent| {
            intent["id"] == id
                && intent["move"].as_str() == Some(move_name.as_str())
                && intent["source"] == "player"
        }),
        "{tick}"
    );
}

#[then(expr = "street tick {string} has no derived pc_rowan kernel intent")]
fn street_no_kernel(w: &mut World, name: String) {
    let _ = w;
    assert_no_pc_kernel(&tick_value(&name));
}

#[then(expr = "street tick {string} rejects the last queued id as {string}")]
fn street_rejects(w: &mut World, name: String, reason: String) {
    let _ = w;
    let id = lock_session()
        .as_ref()
        .expect("no street session")
        .last_queued_id
        .clone();
    let tick = tick_value(&name);
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

#[then(expr = "street tick {string} has no pc_rowan location effect")]
fn street_no_location(w: &mut World, name: String) {
    let _ = w;
    assert_eq!(pc_location_count(&tick_value(&name)), 0);
}

#[then(expr = "the street player stdout gains the street place")]
fn street_gains_place(w: &mut World) {
    let _ = w;
    let (stdout, mark) = player_stdout();
    let full = wait_has(&stdout, "\"place\":\"docks_street\"", Duration::from_secs(5));
    store_added(slice_added(&full, &mark));
}

#[then(expr = "the street addition contains kind perceive")]
fn street_addition_perceive(w: &mut World) {
    let _ = w;
    let added = lock_session()
        .as_ref()
        .expect("no street session")
        .added
        .clone();
    assert!(added.contains("\"kind\":\"perceive\""), "{added}");
}

#[then(expr = "street world_time advanced by {int} from the mark")]
fn street_advanced(w: &mut World, delta: i64) {
    let _ = w;
    let mark = lock_session()
        .as_ref()
        .expect("no street session")
        .world_time_mark;
    assert_eq!(current_now().world_time, mark + delta as u64);
}

#[then(expr = "the street scene is docks_street")]
fn street_scene(w: &mut World) {
    let _ = w;
    let text = player_text();
    assert_street(&text, &current_now());
}

#[then(expr = "street tick {string} ran from {int} to {int}")]
fn street_tick_span(w: &mut World, name: String, before: i64, after: i64) {
    let _ = w;
    let tick = tick_value(&name);
    assert_eq!(tick["world_time_before"].as_i64(), Some(before), "{tick}");
    assert_eq!(tick["world_time_after"].as_i64(), Some(after), "{tick}");
}

#[then(expr = "street tick {string} moved Rowan from {word} to {word} on the last queued id")]
fn street_moved(w: &mut World, name: String, from: String, to: String) {
    let _ = w;
    assert_eq!(from, "docks_shop");
    assert_eq!(to, "docks_street");
    let id = lock_session()
        .as_ref()
        .expect("no street session")
        .last_queued_id
        .clone();
    assert_street_effect(&tick_value(&name), &id);
}

#[then(expr = "the street player gained a resume and no queued line")]
fn street_live_resume(w: &mut World) {
    let _ = w;
    let (stdout, mark) = player_stdout();
    let want = mark.matches("\"kind\":\"resume\"").count() + 1;
    let full = wait_count(&stdout, "\"kind\":\"resume\"", want, Duration::from_secs(5));
    let added = slice_added(&full, &mark);
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
    assert!(!added.contains("\"kind\":\"queued\""), "{added}");
    store_added(added);
}

#[then(expr = "street tick {string} has one player rejection {string}")]
fn street_one_rejection(w: &mut World, name: String, reason: String) {
    let _ = w;
    let tick = tick_value(&name);
    let refused = player_rejections(&tick);
    assert_eq!(refused.len(), 1, "{tick}");
    assert_eq!(refused[0]["reason"].as_str(), Some(reason.as_str()));
}

#[then(expr = "street tick {string} has no pc_rowan travel intent")]
fn street_no_travel_intent(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&name);
    assert!(tick["intents"].as_array().unwrap().iter().all(|intent| {
        intent["actor_id"] != "pc_rowan" || intent["move"] != "travel"
    }));
}

#[then(expr = "the street player sees the street with resume and perceive and no queued line")]
fn street_sees_live(w: &mut World) {
    let _ = w;
    let (stdout, mark) = player_stdout();
    let full = wait_has(&stdout, "\"place\":\"docks_street\"", Duration::from_secs(5));
    let added = slice_added(&full, &mark);
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
    assert!(added.contains("\"kind\":\"perceive\""), "{added}");
    assert!(!added.contains("\"kind\":\"queued\""), "{added}");
    store_added(added);
}

#[then(
    expr = "street tick {string} has a player travel to {word} with a location effect from {word}"
)]
fn street_live_travel(w: &mut World, name: String, to: String, from: String) {
    let _ = w;
    assert_eq!(to, "docks_street");
    assert_eq!(from, "docks_shop");
    let tick = tick_value(&name);
    let travel = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|intent| {
            intent["actor_id"] == "pc_rowan"
                && intent["move"] == "travel"
                && intent["source"] == "player"
        })
        .unwrap_or_else(|| panic!("no travel intent in {tick}"));
    assert_eq!(travel["target"]["place"], "docks_street");
    let id = travel["id"].as_str().unwrap().to_string();
    assert_street_effect(&tick, &id);
}

#[then(expr = "the street session stops")]
fn street_stops(w: &mut World) {
    drop_session();
    w.pack = None;
}
