use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_kernel::expand::expand;
use spookery_kernel::model::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::Reason;

use crate::support::{self, text_of, Proc};
use crate::World;

// Procs, the GM socket, and the in-memory shard cannot live on World.
struct Hold {
    lock: Option<support::PortLock>,
    server: Option<Proc>,
    player: Option<Proc>,
    gm: Option<TcpStream>,
    shard: Option<spookery_kernel::store::Shard>,
    pack: Option<PathBuf>,
    twin: Option<PathBuf>,
    twin_shard: Option<spookery_kernel::store::Shard>,
    live_now: Option<spookery_kernel::model::Now>,
    reason: Option<Reason>,
    perceive_len: usize,
    mark: usize,
    captured_stdout: String,
    captured_stderr: String,
    gm_lines: Vec<String>,
}

static HOLD: Mutex<Hold> = Mutex::new(Hold {
    lock: None,
    server: None,
    player: None,
    gm: None,
    shard: None,
    pack: None,
    twin: None,
    twin_shard: None,
    live_now: None,
    reason: None,
    perceive_len: 0,
    mark: 0,
    captured_stdout: String::new(),
    captured_stderr: String::new(),
    gm_lines: Vec::new(),
});

fn hold() -> std::sync::MutexGuard<'static, Hold> {
    HOLD.lock().unwrap_or_else(|err| err.into_inner())
}

fn stop_proc(proc: &mut Proc) {
    let _ = proc.child.kill();
    let _ = proc.child.wait();
}

fn clear() {
    let mut slot = hold();
    let mut server = slot.server.take();
    let mut player = slot.player.take();
    let gm = slot.gm.take();
    let shard = slot.shard.take();
    let twin_shard = slot.twin_shard.take();
    let pack = slot.pack.take();
    let twin = slot.twin.take();
    let lock = slot.lock.take();
    slot.live_now = None;
    slot.reason = None;
    slot.perceive_len = 0;
    slot.mark = 0;
    slot.captured_stdout.clear();
    slot.captured_stderr.clear();
    slot.gm_lines.clear();
    drop(slot);
    drop(gm);
    if let Some(proc) = player.as_mut() {
        stop_proc(proc);
    }
    if let Some(proc) = server.as_mut() {
        stop_proc(proc);
    }
    drop(player);
    drop(server);
    drop(shard);
    drop(twin_shard);
    if let Some(path) = &pack {
        let _ = fs::remove_dir_all(path);
    }
    if let Some(path) = &twin {
        let _ = fs::remove_dir_all(path);
    }
    drop(lock);
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

fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => panic!("bad hex {byte}"),
    }
}

fn hex_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    assert!(bytes.len() % 2 == 0, "odd hex");
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        out.push((hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]));
    }
    out
}

fn scratch_bytes(w: &World, key: &str) -> Vec<u8> {
    let text = w.scratch.get(key).unwrap_or_else(|| panic!("missing {key}"));
    hex_decode(text)
}

fn put_bytes(w: &mut World, key: &str, bytes: &[u8]) {
    w.scratch.insert(key.to_string(), hex_encode(bytes));
}

fn pack_of(w: &World) -> &Path {
    w.pack.as_ref().expect("attempt pack")
}

fn open_now(w: &World) -> spookery_kernel::model::Now {
    spookery_kernel::open_shard(pack_of(w))
        .unwrap_or_else(|err| panic!("open: {err}"))
        .now()
        .clone()
}

fn must_err(result: Result<spookery::seat::Turn, Reason>) -> Reason {
    match result {
        Err(code) => code,
        Ok(_) => panic!("attempt was accepted"),
    }
}

fn intent(move_name: &str) -> Intent {
    Intent {
        id: "a".to_string(),
        tick_index: 1,
        actor_id: "pc_rowan".to_string(),
        actor_kind: ActorKind::Player,
        source: Source::Player,
        move_name: move_name.to_string(),
        target: Target::Empty,
        args: IntentArgs {
            check: Some("grit".to_string()),
            check_target: Some(3),
            ..IntentArgs::default()
        },
        rationale_code: String::new(),
    }
}

fn tick_text(pack: &Path) -> String {
    fs::read_to_string(pack.join("state/log/tick-0001.jsonl"))
        .unwrap_or_else(|err| panic!("tick: {err}"))
}

fn tick_value(pack: &Path) -> Value {
    let text = tick_text(pack);
    serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("json {err}: {text}"))
}

fn tick_files(pack: &Path) -> Vec<String> {
    let log = pack.join("state/log");
    if !log.is_dir() {
        return Vec::new();
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&log).unwrap_or_else(|err| panic!("read {}: {err}", log.display())) {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        if name.starts_with("tick-") && name.ends_with(".jsonl") {
            names.push(name.into_owned());
        }
    }
    names.sort();
    names
}

fn player_checks(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.starts_with("{\"kind\":\"check\""))
        .map(|line| line.to_string())
        .collect()
}

fn assert_roll(params: &Value, target: i64) {
    let faces = params["faces"].as_array().unwrap();
    assert_eq!(faces.len(), 2, "{params}");
    let sum = faces.iter().map(|face| face.as_i64().unwrap()).sum::<i64>();
    assert!(
        faces
            .iter()
            .all(|face| (1..=6).contains(&face.as_i64().unwrap())),
        "{params}"
    );
    assert_eq!(params["target"], target);
    assert_eq!(params["stat"], 1);
    assert_eq!(params["total"], sum + params["stat"].as_i64().unwrap());
    assert!(params["success"].is_boolean());
    assert_eq!(
        params["success"],
        params["total"].as_i64().unwrap() >= target
    );
}

fn assert_event_order(event: &str) {
    let mut cursor = 0;
    for key in ["kind", "faces", "stat", "total", "target", "success"] {
        let found = event[cursor..]
            .find(&format!("\"{key}\""))
            .unwrap_or_else(|| panic!("missing {key} in {event}"));
        cursor += found + 1;
    }
}

fn kernel_moves(tick: &Value) -> Vec<(String, String)> {
    tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["source"] == "kernel")
        .map(|intent| {
            (
                intent["actor_id"].as_str().unwrap().to_string(),
                intent["move"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn assert_no_roll_keys(value: &Value, in_args: bool) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if !in_args && matches!(key.as_str(), "faces" | "stat" | "total" | "success") {
                    panic!("roll field {key} in {value}");
                }
                if !in_args && key == "target" && child.is_number() {
                    panic!("roll target outside args in {value}");
                }
                assert_no_roll_keys(child, in_args || key == "args");
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_no_roll_keys(item, in_args);
            }
        }
        _ => {}
    }
}

fn assert_commit_keys(pack: &Path) {
    let raw = tick_text(pack);
    let tick = tick_value(pack);
    let mut keys: Vec<_> = tick.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "effects",
            "intents",
            "last_tick_at",
            "presence_humans",
            "rejected",
            "seed",
            "state_rev_after",
            "state_rev_before",
            "tick_index",
            "tool_trace",
            "world_time_after",
            "world_time_before",
        ]
    );
    let mut cursor = 0;
    for key in [
        "tick_index",
        "state_rev_before",
        "state_rev_after",
        "world_time_before",
        "world_time_after",
        "presence_humans",
        "seed",
        "last_tick_at",
        "intents",
        "rejected",
        "effects",
        "tool_trace",
    ] {
        let found = raw[cursor..]
            .find(&format!("\"{key}\""))
            .unwrap_or_else(|| panic!("missing {key} in {raw}"));
        cursor += found + 1;
    }
    assert_no_roll_keys(&tick, false);
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

fn event_params(line: &str) -> Value {
    let value: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}: {line}"));
    value["params"].clone()
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

fn read_until(stream: &mut TcpStream, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    let mut lines = Vec::new();
    let start = Instant::now();
    while !done(&lines) {
        if start.elapsed() > Duration::from_secs(5) {
            panic!("gm lines stalled: {lines:?}");
        }
        lines.push(read_lf(stream, Duration::from_secs(5)));
    }
    lines
}

fn drain_ready(stream: &mut TcpStream) -> Vec<String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(300)))
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

fn finish_gm(stream: &mut TcpStream, mut lines: Vec<String>) -> Vec<String> {
    lines.extend(drain_ready(stream));
    lines
}

fn with_gm<T>(f: impl FnOnce(&mut TcpStream) -> T) -> T {
    let mut stream = hold().gm.take().expect("no gm session");
    let result = f(&mut stream);
    hold().gm = Some(stream);
    result
}

fn commit_gm(wait_for_result: bool) {
    let lines = with_gm(|gm| {
        let _queued = drain_ready(gm);
        gm.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"commit_turn\",\"arguments\":{\"include_tick\":true}}}\n")
            .unwrap();
        gm.flush().unwrap();
        let lines = read_until(gm, |lines| {
            let faces = lines.iter().any(|line| line.contains("\"faces\""));
            if wait_for_result {
                faces && lines.iter().any(|line| line.contains("\"id\":2"))
            } else {
                faces
            }
        });
        finish_gm(gm, lines)
    });
    hold().gm_lines = lines;
}

fn send_and_capture(line: &str, needle: &str) -> String {
    let stdout = {
        let mut slot = hold();
        let player = slot.player.as_mut().expect("no player");
        support::write_stdin(player, line);
        player.stdout.clone()
    };
    support::wait_has(&stdout, needle, Duration::from_secs(5))
}

fn send_and_count(line: &str, needle: &str, count: usize) -> String {
    let stdout = {
        let mut slot = hold();
        let player = slot.player.as_mut().expect("no player");
        support::write_stdin(player, line);
        player.stdout.clone()
    };
    support::wait_count(&stdout, needle, count, Duration::from_secs(5))
}

fn remember_stdout(text: String, set_mark: bool) {
    let mut slot = hold();
    if set_mark {
        slot.mark = text.len();
    }
    slot.captured_stdout = text;
}

fn gm_check_line() -> String {
    let lines = hold().gm_lines.clone();
    let events: Vec<_> = lines
        .into_iter()
        .filter(|line| line.contains("\"kind\":\"check\""))
        .collect();
    assert_eq!(events.len(), 1, "{events:?}");
    events.into_iter().next().unwrap()
}

fn gm_params() -> Value {
    event_params(&gm_check_line())
}

#[given(expr = "the attempt port is locked and the shard is listening")]
fn listen(w: &mut World) {
    clear();
    let lock = support::lock_port();
    let pack = support::temp_pack("attempt");
    let sheet = fs::read(pack.join("state/actors/pc_rowan.yaml")).unwrap();
    put_bytes(w, "attempt-sheet", &sheet);
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    w.pack = Some(pack.clone());
    let mut slot = hold();
    slot.lock = Some(lock);
    slot.server = Some(server);
    slot.pack = Some(pack);
}

#[when(expr = "an unseated attempt shard receives attempt grit 3")]
fn unseated_grit(w: &mut World) {
    clear();
    let pack = support::temp_pack("shape");
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    put_bytes(w, "attempt-before", &before);
    w.pack = Some(pack.clone());
    let mut shard = spookery_kernel::open_shard(&pack).unwrap();
    assert_eq!(shard.now().presence.humans, 0);
    let live = must_err(spookery::seat::turn(
        &mut shard,
        "pc_rowan",
        "attempt grit 3",
        false,
    ));
    let mut slot = hold();
    slot.reason = Some(live);
    slot.shard = Some(shard);
    slot.pack = Some(pack);
}

#[then(expr = "attempt presence humans is {int}")]
fn humans_is(w: &mut World, n: u64) {
    assert_eq!(open_now(w).presence.humans as u64, n);
}

#[then(expr = "the attempt refusal is LIVE_ONLY")]
fn refusal_live(_w: &mut World) {
    assert_eq!(hold().reason, Some(Reason::LiveOnly));
}

#[then(expr = "the attempt refusal is NOT_IN_SPEC")]
fn refusal_spec(_w: &mut World) {
    assert_eq!(hold().reason, Some(Reason::NotInSpec));
}

#[then(expr = "the attempt queue is empty")]
fn queue_empty(_w: &mut World) {
    assert!(hold().shard.as_ref().unwrap().queued_intents().is_empty());
}

#[then(expr = "the attempt pack has no session.jsonl")]
fn no_session(w: &mut World) {
    assert!(!pack_of(w).join("state/log/session.jsonl").exists());
}

#[then(expr = "the attempt pack has no tick-0001.jsonl")]
fn no_tick_one(w: &mut World) {
    assert!(!pack_of(w).join("state/log/tick-0001.jsonl").exists());
}

#[then(expr = "the attempt pack has no tick file")]
fn no_tick_file(w: &mut World) {
    assert!(tick_files(pack_of(w)).is_empty());
}

#[then(expr = "the attempt now.yaml matches the before bytes")]
fn now_before(w: &mut World) {
    let bytes = fs::read(pack_of(w).join("state/now.yaml")).unwrap();
    assert_eq!(bytes, scratch_bytes(w, "attempt-before"));
}

#[then(expr = "the attempt now.yaml matches the seated bytes")]
fn now_seated(w: &mut World) {
    let bytes = fs::read(pack_of(w).join("state/now.yaml")).unwrap();
    assert_eq!(bytes, scratch_bytes(w, "attempt-seated"));
}

#[when(expr = "the attempt shard seats pc_rowan")]
fn seat_rowan(w: &mut World) {
    let pack = {
        let mut slot = hold();
        let shard = slot.shard.as_mut().unwrap();
        spookery::seat::seat_player(shard, "pc_rowan").unwrap();
        slot.pack.clone().unwrap()
    };
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();
    put_bytes(w, "attempt-seated", &seated);
}

#[then(expr = "attempt and attempt grit are SCHEMA and write nothing")]
fn schema_lines(w: &mut World) {
    let seated = scratch_bytes(w, "attempt-seated");
    let pack = pack_of(w).to_path_buf();
    let mut slot = hold();
    let shard = slot.shard.as_mut().unwrap();
    for line in ["attempt", "attempt grit"] {
        let err = must_err(spookery::seat::turn(shard, "pc_rowan", line, false));
        assert_eq!(err, Reason::Schema, "{line}");
        assert!(shard.queued_intents().is_empty(), "{line}");
        assert_eq!(
            fs::read(pack.join("state/now.yaml")).unwrap(),
            seated,
            "{line}"
        );
    }
}

#[when(expr = "the seated attempt shard receives attempt sneak shadow")]
fn in_memory_shadow(_w: &mut World) {
    let mut slot = hold();
    let shard = slot.shard.as_mut().unwrap();
    let bad = must_err(spookery::seat::turn(
        shard,
        "pc_rowan",
        "attempt sneak shadow",
        false,
    ));
    slot.reason = Some(bad);
}

#[then(expr = "expand of move attempt is empty")]
fn expand_attempt(_w: &mut World) {
    let attempt = intent("attempt");
    let slot = hold();
    let shard = slot.shard.as_ref().unwrap();
    assert!(expand(shard.canon(), shard.now(), &attempt)
        .unwrap()
        .is_empty());
}

#[then(expr = "expand of moves attack, give, and take is NOT_IN_SPEC")]
fn expand_others(_w: &mut World) {
    let attempt = intent("attempt");
    let slot = hold();
    let shard = slot.shard.as_ref().unwrap();
    for name in ["attack", "give", "take"] {
        let mut other = attempt.clone();
        other.move_name = name.to_string();
        assert_eq!(
            expand(shard.canon(), shard.now(), &other).unwrap_err(),
            Reason::NotInSpec,
            "{name}"
        );
    }
}

#[when(expr = "the attempt player connects and perceives")]
fn player_connects(w: &mut World) {
    let pack = support::pack_arg(pack_of(w));
    let player = support::spawn(&["play", &pack]);
    assign_job(&player.child);
    let stdout = support::wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let mut slot = hold();
    slot.perceive_len = stdout.len();
    slot.player = Some(player);
}

#[when(expr = "the attempt player sends attempt grit 3 with no GM")]
fn send_grit_live(_w: &mut World) {
    let text = send_and_capture("attempt grit 3", "{\"kind\":\"check\",\"success\":true}");
    remember_stdout(text, false);
}

#[when(expr = "the attempt player sends attempt grit 14")]
fn send_grit_fail(_w: &mut World) {
    let text = send_and_capture("attempt grit 14", "{\"kind\":\"check\",\"success\":false}");
    remember_stdout(text, false);
}

#[when(expr = "the attempt player sends attempt grit 3 while a GM is seated")]
fn send_grit_queued(_w: &mut World) {
    let text = send_and_capture("attempt grit 3", "\"kind\":\"queued\"");
    remember_stdout(text, true);
}

#[when(expr = "the attempt player sends attempt nope 5")]
fn send_nope(_w: &mut World) {
    let text = send_and_capture("attempt nope 5", "\"kind\":\"queued\"");
    remember_stdout(text, false);
}

#[when(expr = "the attempt player sends a second queued attempt grit 3")]
fn send_second_grit(_w: &mut World) {
    let text = send_and_count("attempt grit 3", "\"kind\":\"queued\"", 2);
    remember_stdout(text, true);
}

#[when(expr = "the attempt player sends attempt sneak shadow")]
fn send_shadow(_w: &mut World) {
    let stderr = {
        let mut slot = hold();
        let player = slot.player.as_mut().unwrap();
        support::write_stdin(player, "attempt sneak shadow");
        player.stderr.clone()
    };
    let text = support::wait_has(&stderr, "NOT_IN_SPEC", Duration::from_secs(5));
    hold().captured_stderr = text;
}

#[when(expr = "the seated attempt now.yaml is recorded")]
fn record_seated(w: &mut World) {
    let bytes = fs::read(pack_of(w).join("state/now.yaml")).unwrap();
    put_bytes(w, "attempt-seated", &bytes);
}

#[when(expr = "an attempt GM session is open")]
fn open_gm(_w: &mut World) {
    let gm = gm_session();
    hold().gm = Some(gm);
}

#[when(expr = "the GM commits the attempt turn")]
fn commit_with_result(_w: &mut World) {
    commit_gm(true);
}

#[when(expr = "the GM commits the attempt turn after the unknown check")]
fn commit_faces_only(_w: &mut World) {
    commit_gm(false);
}

#[when(expr = "a twin shard seats pc_rowan queues wait and commits")]
fn twin_wait(w: &mut World) {
    let now = open_now(w);
    let twin = support::temp_pack("wait-twin");
    let mut waited = spookery_kernel::open_shard(&twin).unwrap();
    spookery::seat::seat_player(&mut waited, "pc_rowan").unwrap();
    spookery::seat::queue_wait(&mut waited, "pc_rowan").unwrap();
    assert!(spookery::seat::commit_queued(&mut waited).unwrap());
    w.scratch
        .insert("attempt-twin".to_string(), twin.display().to_string());
    let mut slot = hold();
    slot.live_now = Some(now);
    slot.twin = Some(twin);
    slot.twin_shard = Some(waited);
}

#[then(expr = "the player checks are one success true")]
fn one_success(_w: &mut World) {
    let stdout = hold().captured_stdout.clone();
    assert_eq!(
        player_checks(&stdout),
        vec!["{\"kind\":\"check\",\"success\":true}".to_string()]
    );
}

#[then(expr = "the player stdout has no faces")]
fn no_faces(_w: &mut World) {
    let stdout = hold().captured_stdout.clone();
    assert!(!stdout.contains("\"faces\""));
}

#[then(expr = "the attempt player stderr does not contain NOT_IN_SPEC")]
fn stderr_clean(_w: &mut World) {
    let stderr = hold().player.as_ref().unwrap().stderr.clone();
    assert!(!text_of(&stderr).contains("NOT_IN_SPEC"));
}

#[then(expr = "attempt world_time is {int}")]
fn world_time_is(w: &mut World, n: u64) {
    assert_eq!(open_now(w).world_time, n);
}

#[then(expr = "attempt tick_index is {int}")]
fn tick_index_is(w: &mut World, n: u64) {
    assert_eq!(open_now(w).tick_index, n);
}

#[then(expr = "pc_rowan.yaml matches the attempt sheet")]
fn sheet_matches(w: &mut World) {
    let bytes = fs::read(pack_of(w).join("state/actors/pc_rowan.yaml")).unwrap();
    assert_eq!(bytes, scratch_bytes(w, "attempt-sheet"));
}

#[then(expr = "tick-0001.jsonl has the attempt commit keys in order and no roll fields")]
fn commit_keys(w: &mut World) {
    assert_commit_keys(pack_of(w));
}

#[then(expr = "the attempt intent args are check grit and target 3")]
fn attempt_args(w: &mut World) {
    let tick = tick_value(pack_of(w));
    let attempt = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|intent| intent["move"] == "attempt")
        .unwrap();
    assert_eq!(attempt["args"]["check"], "grit");
    assert_eq!(attempt["args"]["target"], 3);
    assert!(attempt["args"]["target"].is_i64());
}

#[then(expr = "the attempt tool_trace contains resolve_check")]
fn trace_has_check(w: &mut World) {
    let tick = tick_value(pack_of(w));
    assert!(tick["tool_trace"]
        .as_array()
        .unwrap()
        .iter()
        .any(|name| name == "resolve_check"));
}

#[then(expr = "assassination_debt is not paused after the attempt")]
fn debt_running(_w: &mut World) {
    let now = hold().live_now.clone().unwrap();
    assert!(!now.clocks.get("assassination_debt").unwrap().paused);
}

#[then(expr = "the twin now matches the attempt now on the v0 paths")]
fn twin_paths(_w: &mut World) {
    let slot = hold();
    let now = slot.live_now.as_ref().unwrap();
    let waited = slot.twin_shard.as_ref().unwrap().now();
    assert_eq!(waited.actors, now.actors);
    assert_eq!(waited.pause, now.pause);
    assert_eq!(waited.clocks, now.clocks);
    assert_eq!(waited.places, now.places);
    assert_eq!(waited.rumors, now.rumors);
    assert_eq!(waited.messages, now.messages);
    assert_eq!(waited.proposals, now.proposals);
    assert_eq!(waited.world_time, now.world_time);
}

#[then(expr = "the twin kernel moves match the attempt tick")]
fn twin_moves(w: &mut World) {
    let twin = hold().twin.clone().unwrap();
    assert_eq!(
        kernel_moves(&tick_value(&twin)),
        kernel_moves(&tick_value(pack_of(w)))
    );
}

#[then(expr = "the player checks are success true then success false")]
fn both_checks(_w: &mut World) {
    let stdout = hold().captured_stdout.clone();
    assert_eq!(
        player_checks(&stdout),
        vec![
            "{\"kind\":\"check\",\"success\":true}".to_string(),
            "{\"kind\":\"check\",\"success\":false}".to_string(),
        ]
    );
}

#[then(expr = "the player stdout after perceive is queued without check or resume")]
fn queued_only(_w: &mut World) {
    let slot = hold();
    let tail = &slot.captured_stdout[slot.perceive_len..];
    assert!(!tail.contains("\"kind\":\"check\""), "{tail}");
    assert!(!tail.contains("\"kind\":\"resume\""), "{tail}");
}

#[then(expr = "the GM sees one check event after the commit result")]
fn one_check_after_result(_w: &mut World) {
    let lines = hold().gm_lines.clone();
    let checks: Vec<_> = lines
        .iter()
        .filter(|line| line.contains("\"kind\":\"check\""))
        .collect();
    assert_eq!(checks.len(), 1, "{lines:?}");
    let result_at = lines
        .iter()
        .position(|line| line.contains("\"id\":2"))
        .unwrap();
    let event_at = lines
        .iter()
        .position(|line| line.contains("\"faces\""))
        .unwrap();
    assert!(result_at < event_at, "{lines:?}");
}

#[then(expr = "the GM sees one check event")]
fn one_check(_w: &mut World) {
    let lines = hold().gm_lines.clone();
    let events: Vec<_> = lines
        .iter()
        .filter(|line| line.contains("\"kind\":\"check\""))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "{lines:?}");
}

#[then(expr = "the commit result has no faces")]
fn result_no_faces(_w: &mut World) {
    let lines = hold().gm_lines.clone();
    let result_at = lines
        .iter()
        .position(|line| line.contains("\"id\":2"))
        .unwrap();
    let result = &lines[result_at];
    assert!(!result.contains("faces"), "{result}");
}

#[then(expr = "the GM check event lists faces before stat total target and success")]
fn event_order(_w: &mut World) {
    let event = gm_check_line();
    assert!(
        event.contains("\"params\":{\"kind\":\"check\",\"faces\":["),
        "{event}"
    );
    assert_event_order(&event);
}

#[then(expr = "the GM roll has two faces stat 1 and target 3")]
fn roll_shape(_w: &mut World) {
    let params = gm_params();
    assert_roll(&params, 3);
}

#[then(expr = "the player check success matches the GM roll")]
fn player_matches(_w: &mut World) {
    let mark = hold().mark;
    let params = gm_params();
    let stdout_slot = hold().player.as_ref().unwrap().stdout.clone();
    let stdout = support::wait_has(
        &stdout_slot,
        "{\"kind\":\"check\",\"success\":",
        Duration::from_secs(5),
    );
    let word = if params["success"] == true {
        "true"
    } else {
        "false"
    };
    assert_eq!(
        player_checks(&stdout[mark..]),
        vec![format!("{{\"kind\":\"check\",\"success\":{word}}}")]
    );
}

#[then(expr = "the player stdout has two queued lines and no check")]
fn two_queued(_w: &mut World) {
    let text = hold().captured_stdout.clone();
    assert!(text.matches("\"kind\":\"queued\"").count() >= 2, "{text}");
    assert!(!text.contains("{\"kind\":\"check\""), "{text}");
}

#[then(expr = "the attempt player stderr is UNKNOWN_KEY")]
fn stderr_unknown(_w: &mut World) {
    let stderr = hold().player.as_ref().unwrap().stderr.clone();
    let text = support::wait_has(&stderr, "UNKNOWN_KEY", Duration::from_secs(5));
    assert_eq!(text.trim(), "UNKNOWN_KEY");
}

#[then(expr = "the attempt player stderr contains NOT_IN_SPEC")]
fn stderr_spec(_w: &mut World) {
    let text = hold().captured_stderr.clone();
    assert!(text.contains("NOT_IN_SPEC"));
}

#[then(expr = "the attempt player is still running")]
fn still_running(_w: &mut World) {
    assert!(hold()
        .player
        .as_mut()
        .unwrap()
        .child
        .try_wait()
        .unwrap()
        .is_none());
}

#[then(expr = "tick-0001.jsonl has no roll fields outside args")]
fn no_roll_fields(w: &mut World) {
    assert_no_roll_keys(&tick_value(pack_of(w)), false);
}

#[then(expr = "tick-0001.jsonl rejects UNKNOWN_KEY and keeps only the grit attempt")]
fn rejected_unknown(w: &mut World) {
    let tick = tick_value(pack_of(w));
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected.iter().any(|row| row["reason"] == "UNKNOWN_KEY"),
        "{tick}"
    );
    assert!(tick["intents"].as_array().unwrap().iter().any(
        |intent| intent["move"] == "attempt" && intent["args"]["check"] == "grit"
    ));
    assert!(!tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|intent| intent["args"]["check"] == "nope"));
}

#[then(expr = "tick-0001.jsonl resolve_check count is 2")]
fn two_resolves(w: &mut World) {
    let tick = tick_value(pack_of(w));
    assert_eq!(
        tick["tool_trace"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|name| *name == "resolve_check")
            .count(),
        2
    );
}

#[then(expr = "the GM faces match a fresh grit resolve_check")]
fn fresh_faces(w: &mut World) {
    let params = gm_params();
    let tick = tick_value(pack_of(w));
    let seed = tick["seed"].as_str().unwrap();
    let rng = spookery_kernel::blake2s_256(seed.as_bytes());
    let mut cursor = 0usize;
    let shard = spookery_kernel::open_shard(pack_of(w)).unwrap();
    let fresh = spookery_kernel::check::resolve_check(
        shard.canon(),
        shard.sheets(),
        &ViewerId::Player("pc_rowan".to_string()),
        "grit",
        3,
        &rng,
        &mut cursor,
    )
    .unwrap();
    let faces: Vec<u32> = params["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|face| face.as_u64().unwrap() as u32)
        .collect();
    assert_eq!(faces, fresh.faces);
    assert_eq!(cursor, fresh.faces.len());
}

#[then(expr = "the attempt session ends")]
fn session_ends(w: &mut World) {
    clear();
    w.pack = None;
}
