use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, text_of, wait_count, wait_has, write_stdin, Proc};
use crate::World;

// The GM socket and live spook processes cannot live on World.
// Fields drop in declaration order: the socket, the player, the server, then the port lock.
struct Hold {
    stream: TcpStream,
    player: Option<Proc>,
    server: Proc,
    #[allow(dead_code)]
    lock: support::PortLock,
    pack: PathBuf,
    before: Vec<u8>,
    seated: Option<Vec<u8>>,
    tape_after_spoof: Option<String>,
    last_lines: Vec<String>,
    last_id: u64,
    next_gm_id: u64,
    queued_id: Option<String>,
}

static HOLD: Mutex<Option<Hold>> = Mutex::new(None);

fn lock_hold() -> std::sync::MutexGuard<'static, Option<Hold>> {
    HOLD.lock().unwrap_or_else(|err| err.into_inner())
}

fn drop_hold() {
    let hold = lock_hold().take();
    let Some(hold) = hold else {
        return;
    };
    let pack = hold.pack.clone();
    drop(hold);
    let _ = fs::remove_dir_all(pack);
}

fn hold() -> std::sync::MutexGuard<'static, Option<Hold>> {
    let guard = lock_hold();
    assert!(guard.is_some(), "no GM say shard");
    guard
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
    let found = value.pointer("/result/structuredContent");
    found
        .cloned()
        .unwrap_or_else(|| panic!("no structured content in {line}"))
}

fn result_line<'a>(lines: &'a [String], id: u64) -> &'a str {
    lines
        .iter()
        .find(|line| line.contains(&format!("\"id\":{id}")))
        .unwrap_or_else(|| panic!("no result {id} in {lines:?}"))
}

fn gm_call(name: &str, arguments: &str) {
    let mut guard = hold();
    let held = guard.as_mut().unwrap();
    let id = held.next_gm_id;
    held.next_gm_id += 1;
    let lines = call(&mut held.stream, id, name, arguments);
    held.last_lines = lines;
    held.last_id = id;
}

fn last_body() -> Value {
    let guard = hold();
    let held = guard.as_ref().unwrap();
    structured(result_line(&held.last_lines, held.last_id))
}

fn tape_text() -> String {
    let pack = hold().as_ref().unwrap().pack.clone();
    fs::read_to_string(pack.join("state/log/session.jsonl"))
        .unwrap_or_else(|err| panic!("read session.jsonl: {err}"))
}

fn now_bytes() -> Vec<u8> {
    let pack = hold().as_ref().unwrap().pack.clone();
    fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"))
}

fn player_stdout() -> std::sync::Arc<std::sync::Mutex<String>> {
    hold()
        .as_ref()
        .unwrap()
        .player
        .as_ref()
        .expect("no GM say player")
        .stdout
        .clone()
}

fn player_stderr() -> std::sync::Arc<std::sync::Mutex<String>> {
    hold()
        .as_ref()
        .unwrap()
        .player
        .as_ref()
        .expect("no GM say player")
        .stderr
        .clone()
}

#[given(expr = "a listening GM say shard")]
fn listen(w: &mut World) {
    drop_hold();
    let lock = support::lock_port();
    let pack = support::temp_pack("say");
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    let stream = gm_session();
    w.pack = Some(pack.clone());
    *lock_hold() = Some(Hold {
        stream,
        player: None,
        server,
        lock,
        pack,
        before,
        seated: None,
        tape_after_spoof: None,
        last_lines: Vec::new(),
        last_id: 0,
        next_gm_id: 3,
        queued_id: None,
    });
}

#[when(expr = "the GM says from the chair at the table")]
fn say_chair(_w: &mut World) {
    gm_call("say", r#"{"address":"table","text":"from the chair"}"#);
}

#[then(expr = "the say result precedes the spook event")]
fn say_precedes_event(_w: &mut World) {
    let guard = hold();
    let held = guard.as_ref().unwrap();
    let said = &held.last_lines;
    let marker = format!("\"id\":{}", held.last_id);
    let said_at = said
        .iter()
        .position(|line| line.contains(&marker))
        .unwrap_or_else(|| panic!("no result in {said:?}"));
    let event_at = said
        .iter()
        .position(|line| line.contains("\"method\":\"spook/event\""))
        .unwrap_or_else(|| panic!("no event in {said:?}"));
    assert!(said_at < event_at, "{said:?}");
}

#[then(expr = "the say body is kind say from gm at the table with text from the chair")]
fn say_body(_w: &mut World) {
    let guard = hold();
    let held = guard.as_ref().unwrap();
    let said_line = result_line(&held.last_lines, held.last_id);
    assert!(
        said_line.contains("\"structuredContent\":{\"kind\":\"say\",\"seq\":1,\"from\":\"gm\""),
        "{said_line}"
    );
    let body = structured(said_line);
    assert_eq!(body["kind"], "say");
    assert_eq!(body["from"], "gm");
    assert_eq!(body["address"], "table");
    assert_eq!(body["text"], "from the chair");
}

#[then(expr = "the say body place and actor are null and recipients are empty")]
fn say_nulls(_w: &mut World) {
    let body = last_body();
    assert!(body["place"].is_null());
    assert!(body["actor"].is_null());
    assert_eq!(body["recipients"], serde_json::json!([]));
}

#[then(expr = "the spook event is the same say and has no id")]
fn say_event(_w: &mut World) {
    let guard = hold();
    let held = guard.as_ref().unwrap();
    let event = held
        .last_lines
        .iter()
        .find(|line| line.contains("\"method\":\"spook/event\""))
        .unwrap_or_else(|| panic!("no event in {:?}", held.last_lines));
    assert!(
        event.contains("\"params\":{\"kind\":\"say\",\"seq\":1,\"from\":\"gm\""),
        "{event}"
    );
    let marker = format!("\"id\":{}", held.last_id);
    assert!(!event.contains(&marker), "{event}");
}

#[then(expr = "session.jsonl contains the gm chair line and no tick")]
fn tape_chair(_w: &mut World) {
    let tape = tape_text();
    assert!(tape.contains("\"from\":\"gm\""), "{tape}");
    assert!(tape.contains("\"text\":\"from the chair\""), "{tape}");
    assert!(!tape.contains("\"tick_index\":1"), "{tape}");
}

#[then(expr = "now.yaml matches the pre-say bytes")]
fn now_before(_w: &mut World) {
    let expected = hold().as_ref().unwrap().before.clone();
    assert_eq!(now_bytes(), expected);
}

#[then(expr = "the GM say shard is still running")]
fn still_running(_w: &mut World) {
    let mut guard = hold();
    let held = guard.as_mut().unwrap();
    assert!(held.server.child.try_wait().unwrap().is_none());
}

#[when(expr = "the GM says a spoofed from")]
fn say_spoof(_w: &mut World) {
    gm_call(
        "say",
        r#"{"address":"table","text":"spoof","from":"player:pc_rowan"}"#,
    );
}

#[then(expr = "the spoofed say is SCHEMA")]
fn spoof_schema(_w: &mut World) {
    let body = last_body();
    assert_eq!(body["code"], "SCHEMA");
}

#[then(expr = "session.jsonl still has one line")]
fn tape_one_line(_w: &mut World) {
    let tape = tape_text();
    assert_eq!(tape.lines().count(), 1, "{tape}");
    hold().as_mut().unwrap().tape_after_spoof = Some(tape);
}

#[when(expr = "the GM calls session_status with an empty object")]
fn status_empty(_w: &mut World) {
    gm_call("session_status", "{}");
}

#[then(expr = "that status has no events")]
fn status_no_events(_w: &mut World) {
    let quiet = last_body();
    assert!(quiet.get("events").is_none(), "{quiet}");
}

#[then(expr = "that status queued is empty and last_seq is 1")]
fn status_queued_empty(_w: &mut World) {
    let quiet = last_body();
    assert_eq!(quiet["queued"], serde_json::json!([]));
    assert_eq!(quiet["last_seq"], 1);
}

#[then(expr = "that status presence and pause are objects")]
fn status_objects(_w: &mut World) {
    let quiet = last_body();
    assert!(quiet["presence"].is_object(), "{quiet}");
    assert!(quiet["pause"].is_object(), "{quiet}");
}

#[then(expr = "that status world_time is {int} and tick_index is {int}")]
fn status_clocks(_w: &mut World, world_time: i64, tick_index: i64) {
    let quiet = last_body();
    assert_eq!(quiet["world_time"], world_time);
    assert_eq!(quiet["tick_index"], tick_index);
}

#[then(expr = "that status has no code")]
fn status_no_code(_w: &mut World) {
    let quiet = last_body();
    assert!(quiet.get("code").is_none(), "{quiet}");
}

#[when(expr = "the GM calls session_status with since 0")]
fn status_since(_w: &mut World) {
    gm_call("session_status", r#"{"since":0}"#);
}

#[then(expr = "that status last_seq is 1 and its one event is the chair say")]
fn status_chair_event(_w: &mut World) {
    let pulled = last_body();
    assert_eq!(pulled["last_seq"], 1);
    let events = pulled["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{pulled}");
    assert_eq!(events[0]["kind"], "say");
    assert_eq!(events[0]["from"], "gm");
    assert_eq!(events[0]["text"], "from the chair");
}

#[then(expr = "that event has no tick_index or world_time")]
fn event_has_no_clocks(_w: &mut World) {
    let pulled = last_body();
    let events = pulled["events"].as_array().unwrap();
    assert!(events[0].get("tick_index").is_none(), "{events:?}");
    assert!(events[0].get("world_time").is_none(), "{events:?}");
}

#[when(expr = "the GM calls session_status with since and an extra key")]
fn status_extra(_w: &mut World) {
    gm_call("session_status", r#"{"since":0,"extra":1}"#);
}

#[then(expr = "that status is SCHEMA and has no last_seq")]
fn status_schema(_w: &mut World) {
    let bad = last_body();
    assert_eq!(bad["code"], "SCHEMA");
    assert!(bad.get("last_seq").is_none(), "{bad}");
}

#[then(expr = "session.jsonl is unchanged after the extra key")]
fn tape_unchanged(_w: &mut World) {
    let expected = hold()
        .as_ref()
        .unwrap()
        .tape_after_spoof
        .clone()
        .expect("tape after the spoof was not recorded");
    assert_eq!(tape_text(), expected);
}

#[when(expr = "the GM say player connects and perceives")]
fn player_connects(_w: &mut World) {
    let pack = hold().as_ref().unwrap().pack.clone();
    let player = support::spawn(&["play", &support::pack_arg(&pack)]);
    assign_job(&player.child);
    let stdout = player.stdout.clone();
    hold().as_mut().unwrap().player = Some(player);
    let hello = wait_has(&stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    assert_eq!(hello.matches("\"kind\":\"perceive\"").count(), 1, "{hello}");
    let seated = fs::read(pack.join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    hold().as_mut().unwrap().seated = Some(seated);
}

#[then(expr = "the player hello is one perceive")]
fn hello_one_perceive(_w: &mut World) {
    let hello = text_of(&player_stdout());
    assert_eq!(hello.matches("\"kind\":\"perceive\"").count(), 1, "{hello}");
}

#[when(expr = "the player calls perceive as player:pc_rowan")]
fn player_perceive(_w: &mut World) {
    let mut guard = hold();
    let player = guard.as_mut().unwrap().player.as_mut().unwrap();
    write_stdin(
        player,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"perceive","arguments":{"as":"player:pc_rowan"}}}"#,
    );
}

#[then(expr = "the player stderr is NOT_IN_SPEC")]
fn player_stderr_not_in_spec(_w: &mut World) {
    let refused = wait_has(&player_stderr(), "NOT_IN_SPEC", Duration::from_secs(5));
    assert_eq!(refused.trim(), "NOT_IN_SPEC");
}

#[then(expr = "the player stdout still has one perceive and no jsonrpc")]
fn player_stdout_no_jsonrpc(_w: &mut World) {
    let after_call = text_of(&player_stdout());
    assert_eq!(
        after_call.matches("\"kind\":\"perceive\"").count(),
        1,
        "{after_call}"
    );
    assert!(!after_call.contains("jsonrpc"), "{after_call}");
}

#[when(expr = "the player calls tools list")]
fn player_tools_list(_w: &mut World) {
    let mut guard = hold();
    let player = guard.as_mut().unwrap().player.as_mut().unwrap();
    write_stdin(
        player,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
}

#[then(expr = "the player stderr is NOT_IN_SPEC twice")]
fn player_stderr_twice(_w: &mut World) {
    let listed = wait_count(&player_stderr(), "NOT_IN_SPEC", 2, Duration::from_secs(5));
    assert_eq!(listed.matches("NOT_IN_SPEC").count(), 2, "{listed}");
}

#[then(expr = "the player stdout does not contain session_status")]
fn player_stdout_no_status(_w: &mut World) {
    let stdout = text_of(&player_stdout());
    assert!(!stdout.contains("session_status"), "{stdout}");
}

#[then(expr = "now.yaml matches the seated bytes")]
fn now_seated(_w: &mut World) {
    let expected = hold()
        .as_ref()
        .unwrap()
        .seated
        .clone()
        .expect("seated now.yaml was not recorded");
    assert_eq!(now_bytes(), expected);
}

#[when(expr = "the player says across the table")]
fn player_says(_w: &mut World) {
    let stdout = {
        let mut guard = hold();
        let player = guard.as_mut().unwrap().player.as_mut().unwrap();
        write_stdin(player, "say across the table");
        player.stdout.clone()
    };
    wait_has(
        &stdout,
        "\"text\":\"across the table\"",
        Duration::from_secs(5),
    );
}

#[when(expr = "the GM says back at you")]
fn say_back(_w: &mut World) {
    gm_call("say", r#"{"address":"table","text":"back at you"}"#);
}

#[then(expr = "the GM say result is from gm")]
fn reply_from_gm(_w: &mut World) {
    let guard = hold();
    let held = guard.as_ref().unwrap();
    let heard_line = result_line(&held.last_lines, held.last_id);
    assert!(heard_line.contains("\"from\":\"gm\""), "{heard_line}");
}

#[then(expr = "the player stdout contains the gm reply back at you")]
fn player_hears_reply(_w: &mut World) {
    let player_out = wait_has(
        &player_stdout(),
        "\"text\":\"back at you\"",
        Duration::from_secs(5),
    );
    assert!(
        player_out.contains(
            "{\"kind\":\"say\",\"from\":\"gm\",\"address\":\"table\",\"text\":\"back at you\"}"
        ),
        "{player_out}"
    );
}

#[when(expr = "the player sends wait")]
fn player_wait(_w: &mut World) {
    let stdout = {
        let mut guard = hold();
        let player = guard.as_mut().unwrap().player.as_mut().unwrap();
        write_stdin(player, "wait");
        player.stdout.clone()
    };
    let queued_out = wait_has(&stdout, "\"kind\":\"queued\"", Duration::from_secs(5));
    let queued_id = queued_out
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["kind"] == "queued")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    hold().as_mut().unwrap().queued_id = Some(queued_id);
}

#[then(expr = "that status lists the player say and the queued wait")]
fn status_lists_queue(_w: &mut World) {
    let queued_id = hold()
        .as_ref()
        .unwrap()
        .queued_id
        .clone()
        .expect("queued id was not recorded");
    let status = last_body();
    assert!(status["last_seq"].as_u64().unwrap() >= 2, "{status}");
    let events = status["events"].as_array().unwrap();
    assert!(
        events
            .iter()
            .any(|event| event["from"] == "player:pc_rowan" && event["text"] == "across the table"),
        "{status}"
    );
    assert_eq!(status["queued"][0]["id"], queued_id);
    assert_eq!(status["queued"][0]["actor_id"], "pc_rowan");
    assert_eq!(status["queued"][0]["move"], "wait");
}

#[then(expr = "the GM say pack has no tick file")]
fn no_tick_file(_w: &mut World) {
    let pack = hold().as_ref().unwrap().pack.clone();
    assert!(pack.join("state/log").read_dir().unwrap().all(|entry| {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        !name.starts_with("tick-")
    }));
}

#[then(expr = "the GM say session ends")]
fn session_ends(w: &mut World) {
    drop_hold();
    w.pack = None;
}
