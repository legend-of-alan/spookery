use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, text_of, wait_exit, wait_has, wait_listen, write_stdin, Proc};
use crate::World;

// Procs and the GM socket cannot live on World. One scenario runs at a time.
// Hold drops the socket, the player, and the server before the port lock.
struct Quiet {
    pack: PathBuf,
    before: Vec<(PathBuf, Vec<u8>)>,
    stdout: String,
    stderr: String,
    code: Option<i32>,
    refused: String,
}

struct Codec {
    pack: PathBuf,
    gm: Option<TcpStream>,
    player: Proc,
    server: Proc,
    tick_index: u64,
    world_time: u64,
    state_rev: u64,
    tape_stored: Vec<u8>,
    opened_text: String,
    last_say_text: String,
    last_player_line: String,
    last_player_stdout: String,
    init: String,
    status_lines: Vec<String>,
    live_event: String,
    tool_text: String,
    say_lines: Vec<String>,
}

struct Hold {
    codec: Option<Codec>,
    quiet: Option<Quiet>,
    #[allow(dead_code)]
    lock: support::PortLock,
}

const PLAYER_KEYS: &[&str] = &["kind", "from", "address", "text"];
const TAPE_KEYS: &[&str] = &[
    "seq",
    "from",
    "address",
    "place",
    "actor",
    "recipients",
    "text",
    "tick_index",
    "world_time",
];
const SAY_KEYS: &[&str] = &[
    "kind",
    "seq",
    "from",
    "address",
    "place",
    "actor",
    "recipients",
    "text",
];
const EVENT_KEYS: &[&str] = &[
    "jsonrpc",
    "method",
    "params",
    "kind",
    "seq",
    "from",
    "address",
    "place",
    "actor",
    "recipients",
    "text",
];

static HOLD: Mutex<Option<Hold>> = Mutex::new(None);

fn lock_hold() -> std::sync::MutexGuard<'static, Option<Hold>> {
    HOLD.lock().unwrap_or_else(|err| err.into_inner())
}

impl Drop for Hold {
    fn drop(&mut self) {
        let quiet = self.quiet.take().map(|row| row.pack);
        let codec_pack = self.codec.take().map(|mut codec| {
            codec.gm.take();
            let _ = codec.player.child.kill();
            let _ = codec.player.child.wait();
            let _ = codec.server.child.kill();
            let _ = codec.server.child.wait();
            let pack = codec.pack.clone();
            drop(codec);
            pack
        });
        if let Some(pack) = quiet {
            let _ = fs::remove_dir_all(pack);
        }
        if let Some(pack) = codec_pack {
            let _ = fs::remove_dir_all(pack);
        }
    }
}

fn drop_hold() {
    lock_hold().take();
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

fn now_of(pack: &Path) -> spookery_kernel::model::Now {
    spookery_kernel::open_shard(pack)
        .unwrap_or_else(|err| panic!("open: {err}"))
        .now()
        .clone()
}

fn pack_snapshot(pack: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    walk(pack, pack, &mut out);
    out.sort();
    out
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

fn player_line(text: &str) -> String {
    format!(
        "{{\"kind\":\"say\",\"from\":\"player:pc_rowan\",\"address\":\"table\",\"text\":\"{text}\"}}"
    )
}

fn tape_record(seq: u64, text: &str, tick_index: u64, world_time: u64) -> String {
    format!(
        "{{\"seq\":{seq},\"from\":\"player:pc_rowan\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"{text}\",\"tick_index\":{tick_index},\"world_time\":{world_time}}}"
    )
}

fn say_shape(seq: u64, from: &str, text: &str) -> String {
    format!(
        "{{\"kind\":\"say\",\"seq\":{seq},\"from\":\"{from}\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"{text}\"}}"
    )
}

fn event_frame(seq: u64, from: &str, text: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"spook/event\",\"params\":{{\"kind\":\"say\",\"seq\":{seq},\"from\":\"{from}\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"{text}\"}}}}"
    )
}

fn tape_path(pack: &Path) -> PathBuf {
    pack.join("state/log/session.jsonl")
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

fn keys_in_order(json: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let bytes = json.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let start = index + 1;
                index += 1;
                let mut escape = false;
                while index < bytes.len() {
                    if escape {
                        escape = false;
                    } else if bytes[index] == b'\\' {
                        escape = true;
                    } else if bytes[index] == b'"' {
                        break;
                    }
                    index += 1;
                }
                let text = json[start..index].to_string();
                index += 1;
                let rest = json[index..].trim_start();
                if rest.starts_with(':') {
                    keys.push(text);
                }
            }
            _ => index += 1,
        }
    }
    keys
}

fn assert_keys(json: &str, expected: &[&str]) {
    let got = keys_in_order(json);
    assert_eq!(got, expected, "{json}");
}

fn structured_object(line: &str) -> &str {
    let marker = "\"structuredContent\":";
    let start = line
        .find(marker)
        .unwrap_or_else(|| panic!("no structuredContent in {line}"))
        + marker.len();
    let rest = &line[start..];
    assert!(rest.starts_with('{'), "{line}");
    let bytes = rest.as_bytes();
    let mut depth = 0;
    let mut in_string = false;
    let mut escape = false;
    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if *byte == b'\\' {
                escape = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..=index];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed structuredContent in {line}");
}

fn codec(guard: &mut Option<Hold>) -> &mut Codec {
    guard
        .as_mut()
        .expect("wire-codec hold")
        .codec
        .as_mut()
        .expect("wire-codec shard")
}

fn quiet(guard: &Option<Hold>) -> &Quiet {
    guard
        .as_ref()
        .expect("wire-codec hold")
        .quiet
        .as_ref()
        .expect("wire-codec quiet pack")
}

#[given(expr = "a wire-codec quiet docks pack and the port lock")]
fn quiet_pack(w: &mut World) {
    drop_hold();
    let lock = support::lock_port();
    let pack = support::temp_pack("c-down");
    let before = pack_snapshot(&pack);
    w.pack = Some(pack.clone());
    *lock_hold() = Some(Hold {
        codec: None,
        quiet: Some(Quiet {
            pack,
            before,
            stdout: String::new(),
            stderr: String::new(),
            code: None,
            refused: String::new(),
        }),
        lock,
    });
}

#[when(expr = "wire-codec gm --stdio runs with nothing listening")]
fn gm_stdio_refused(_w: &mut World) {
    let refused = TcpStream::connect("127.0.0.1:9737")
        .unwrap_err()
        .to_string();
    let pack = {
        let guard = lock_hold();
        quiet(&guard).pack.clone()
    };
    let mut bridge = support::spawn(&["gm", &support::pack_arg(&pack), "--stdio"]);
    assign_job(&bridge.child);
    let status = wait_exit(&mut bridge, Duration::from_secs(5));
    let stdout = text_of(&bridge.stdout);
    let stderr = text_of(&bridge.stderr);
    let mut guard = lock_hold();
    let row = guard
        .as_mut()
        .expect("wire-codec hold")
        .quiet
        .as_mut()
        .expect("wire-codec quiet pack");
    row.stdout = stdout;
    row.stderr = stderr;
    row.code = status.code();
    row.refused = refused;
}

#[then(expr = "wire-codec gm --stdio exits 1")]
fn gm_stdio_exits_1(_w: &mut World) {
    let guard = lock_hold();
    assert_eq!(quiet(&guard).code, Some(1));
}

#[then(expr = "wire-codec gm --stdio stdout is empty")]
fn gm_stdio_stdout_empty(_w: &mut World) {
    let guard = lock_hold();
    let stdout = &quiet(&guard).stdout;
    assert!(stdout.is_empty(), "{stdout}");
}

#[then(expr = "wire-codec gm --stdio stderr is the refused connect error")]
fn gm_stdio_stderr_is_refused(_w: &mut World) {
    let guard = lock_hold();
    let row = quiet(&guard);
    assert_eq!(row.stderr.trim(), row.refused);
}

#[then(expr = "wire-codec gm --stdio stderr does not contain NOT_IN_SPEC")]
fn gm_stdio_stderr_not_in_spec(_w: &mut World) {
    let guard = lock_hold();
    let stderr = &quiet(&guard).stderr;
    assert!(!stderr.contains("NOT_IN_SPEC"), "{stderr}");
}

#[then(expr = "the wire-codec quiet pack bytes are unchanged")]
fn quiet_pack_unchanged(_w: &mut World) {
    let guard = lock_hold();
    let row = quiet(&guard);
    assert_eq!(pack_snapshot(&row.pack), row.before);
}

#[given(expr = "a wire-codec docks pack with spook run listening and a seated player")]
fn codec_pack(w: &mut World) {
    let quiet_pack = {
        let guard = lock_hold();
        quiet(&guard).pack.clone()
    };
    let _ = fs::remove_dir_all(&quiet_pack);
    let pack = support::temp_pack("c-codec");
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    wait_listen(&mut server);
    let player = seat_player(&pack);
    let clock = now_of(&pack);
    w.pack = Some(pack.clone());
    let mut guard = lock_hold();
    let hold = guard.as_mut().expect("wire-codec hold");
    hold.codec = Some(Codec {
        pack,
        gm: None,
        player,
        server,
        tick_index: clock.tick_index,
        world_time: clock.world_time,
        state_rev: clock.state_rev,
        tape_stored: Vec::new(),
        opened_text: String::new(),
        last_say_text: String::new(),
        last_player_line: String::new(),
        last_player_stdout: String::new(),
        init: String::new(),
        status_lines: Vec::new(),
        live_event: String::new(),
        tool_text: String::new(),
        say_lines: Vec::new(),
    });
}

#[when(expr = "the wire-codec player says {string}")]
fn player_says(_w: &mut World, text: String) {
    let line = player_line(&text);
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    if row.opened_text.is_empty() {
        row.opened_text = text.clone();
    }
    row.last_say_text = text.clone();
    write_stdin(&mut row.player, &format!("say {text}"));
    let said = wait_has(&row.player.stdout, &line, Duration::from_secs(5));
    row.last_player_line = line;
    row.last_player_stdout = said;
}

#[then(expr = "the wire-codec player shows that v0.1 say line once")]
fn player_line_once(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let line = row.last_player_line.clone();
    let said = row.last_player_stdout.clone();
    assert_eq!(
        said.lines().filter(|item| *item == line).count(),
        1,
        "{said}"
    );
}

#[then(expr = "that wire-codec player say keeps keys kind, from, address, text")]
fn player_keys(_w: &mut World) {
    let mut guard = lock_hold();
    let line = codec(&mut guard).last_player_line.clone();
    assert_keys(&line, PLAYER_KEYS);
}

#[then(expr = "the wire-codec tape is one line for that say at the seated clock")]
fn tape_one_line(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let line = tape_record(1, &row.last_say_text, row.tick_index, row.world_time);
    let path = tape_path(&row.pack);
    let body = fs::read_to_string(&path).unwrap();
    assert_eq!(body, format!("{line}\n"));
    row.tape_stored = fs::read(&path).unwrap();
}

#[then(
    expr = "that wire-codec tape line keeps keys seq, from, address, place, actor, recipients, text, tick_index, world_time"
)]
fn tape_keys(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let line = tape_record(1, &row.last_say_text, row.tick_index, row.world_time);
    assert_keys(&line, TAPE_KEYS);
}

#[then(expr = "wire-codec world_time and state_rev are unchanged")]
fn clock_unchanged(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let now = now_of(&row.pack);
    assert_eq!(now.world_time, row.world_time);
    assert_eq!(now.state_rev, row.state_rev);
}

#[when(expr = "a wire-codec GM socket sends initialize before any other method")]
fn gm_initialize(_w: &mut World) {
    let mut gm = TcpStream::connect("127.0.0.1:9737").unwrap();
    gm.set_nodelay(true).unwrap();
    gm.write_all(b"gm\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"proof\",\"version\":\"0\"}}}\n")
        .unwrap();
    gm.flush().unwrap();
    let init = read_lf(&mut gm, Duration::from_secs(5));
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    row.init = init;
    row.gm = Some(gm);
}

#[then(expr = "the wire-codec initialize result has id 1 and a result")]
fn init_result(_w: &mut World) {
    let mut guard = lock_hold();
    let init = codec(&mut guard).init.clone();
    assert!(init.contains("\"id\":1"), "{init}");
    assert!(init.contains("\"result\""), "{init}");
}

#[then(expr = "the wire-codec initialize line has no tape bytes")]
fn init_no_tape(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let init = row.init.clone();
    let opened = row.opened_text.clone();
    assert!(!init.contains("spook/event"), "{init}");
    assert!(!init.contains(&opened), "{init}");
    assert!(!init.contains("\"seq\""), "{init}");
}

#[then(expr = "the wire-codec tape bytes are still the bytes from before initialize")]
fn tape_bytes_unchanged(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let path = tape_path(&row.pack);
    assert_eq!(fs::read(&path).unwrap(), row.tape_stored);
}

#[then(expr = "the wire-codec tape still has 1 line")]
fn tape_still_one(_w: &mut World) {
    let mut guard = lock_hold();
    let path = tape_path(&codec(&mut guard).pack);
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 1);
}

#[when(expr = "the wire-codec GM sends notifications\\/initialized")]
fn gm_notifications(_w: &mut World) {
    let mut guard = lock_hold();
    let gm = codec(&mut guard).gm.as_mut().expect("wire-codec GM");
    gm.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .unwrap();
    gm.flush().unwrap();
}

#[when(expr = "the wire-codec GM calls session_status with since 0")]
fn gm_session_status(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let lines = {
        let gm = row.gm.as_mut().expect("wire-codec GM");
        call(gm, 2, "session_status", r#"{"since":0}"#)
    };
    row.status_lines = lines;
}

#[then(expr = "wire-codec session_status events are the scrollback say")]
fn scrollback_events(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let pulled = result_line(&row.status_lines, 2).to_string();
    let say_object = say_shape(1, "player:pc_rowan", &row.opened_text);
    let status_object = structured_object(&pulled).to_string();
    assert!(
        status_object.contains(&format!("\"events\":[{say_object}]")),
        "{status_object}"
    );
}

#[then(
    expr = "that wire-codec scrollback object keeps keys kind, seq, from, address, place, actor, recipients, text"
)]
fn scrollback_keys(_w: &mut World) {
    let mut guard = lock_hold();
    let opened = codec(&mut guard).opened_text.clone();
    let say_object = say_shape(1, "player:pc_rowan", &opened);
    assert_keys(&say_object, SAY_KEYS);
}

#[then(expr = "wire-codec session_status last_seq is 1 and events length is 1")]
fn scrollback_len(_w: &mut World) {
    let mut guard = lock_hold();
    let pulled = result_line(&codec(&mut guard).status_lines, 2).to_string();
    let status = structured(&pulled);
    assert_eq!(status["last_seq"], 1);
    assert_eq!(status["events"].as_array().unwrap().len(), 1);
}

#[then(expr = "the seated wire-codec GM receives that say as a spook\\/event")]
fn gm_receives_say(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let text = row.last_say_text.clone();
    let event = read_lf(
        row.gm.as_mut().expect("wire-codec GM"),
        Duration::from_secs(5),
    );
    let frame = event_frame(2, "player:pc_rowan", &text);
    assert_eq!(event, frame);
    row.live_event = event;
}

#[then(
    expr = "that wire-codec live event keeps keys jsonrpc, method, params, kind, seq, from, address, place, actor, recipients, text"
)]
fn live_keys(_w: &mut World) {
    let mut guard = lock_hold();
    let event = codec(&mut guard).live_event.clone();
    assert_keys(&event, EVENT_KEYS);
}

#[then(expr = "that wire-codec live event has no id")]
fn live_no_id(_w: &mut World) {
    let mut guard = lock_hold();
    let event = codec(&mut guard).live_event.clone();
    assert!(!event.contains("\"id\""), "{event}");
}

#[then(expr = "the wire-codec tape ends with that say and has 2 lines")]
fn tape_two_lines(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let live_tape = format!(
        "{}\n",
        tape_record(2, &row.last_say_text, row.tick_index, row.world_time)
    );
    let tape_now = fs::read_to_string(tape_path(&row.pack)).unwrap();
    assert!(tape_now.ends_with(&live_tape), "{tape_now}");
    assert_eq!(tape_now.lines().count(), 2, "{tape_now}");
}

#[when(expr = "the wire-codec GM calls say addressed to the table with text {string}")]
fn gm_say(_w: &mut World, text: String) {
    let arguments = format!(
        "{{\"address\":\"table\",\"text\":{}}}",
        serde_json::to_string(&text).unwrap()
    );
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    row.tool_text = text;
    let lines = {
        let gm = row.gm.as_mut().expect("wire-codec GM");
        call(gm, 3, "say", &arguments)
    };
    row.say_lines = lines;
}

#[then(expr = "the wire-codec say tool result precedes its spook\\/event")]
fn say_precedes_event(_w: &mut World) {
    let mut guard = lock_hold();
    let lines = codec(&mut guard).say_lines.clone();
    let said_at = lines
        .iter()
        .position(|line| line.contains("\"id\":3"))
        .unwrap();
    let event_at = lines
        .iter()
        .position(|line| line.contains("\"method\":\"spook/event\""))
        .unwrap();
    assert!(said_at < event_at, "{lines:?}");
    assert!(lines
        .iter()
        .take(event_at)
        .all(|line| !line.contains("\"method\":\"spook/event\"")));
}

#[then(expr = "the wire-codec tool object is that chair say")]
fn tool_object_is_chair(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let said_at = row
        .say_lines
        .iter()
        .position(|line| line.contains("\"id\":3"))
        .unwrap();
    let tool_raw = structured_object(&row.say_lines[said_at]).to_string();
    let tool_object = say_shape(3, "gm", &row.tool_text);
    assert_eq!(tool_raw, tool_object);
}

#[then(
    expr = "that wire-codec tool object keeps keys kind, seq, from, address, place, actor, recipients, text"
)]
fn tool_keys(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let said_at = row
        .say_lines
        .iter()
        .position(|line| line.contains("\"id\":3"))
        .unwrap();
    let tool_raw = structured_object(&row.say_lines[said_at]).to_string();
    assert_keys(&tool_raw, SAY_KEYS);
}

#[then(expr = "the wire-codec spook\\/event after the tool result is that chair frame")]
fn tool_event_frame(_w: &mut World) {
    let mut guard = lock_hold();
    let row = codec(&mut guard);
    let event_at = row
        .say_lines
        .iter()
        .position(|line| line.contains("\"method\":\"spook/event\""))
        .unwrap();
    let frame = event_frame(3, "gm", &row.tool_text);
    assert_eq!(row.say_lines[event_at], frame);
}

#[then(expr = "the wire-codec tape has 3 lines")]
fn tape_three_lines(_w: &mut World) {
    let mut guard = lock_hold();
    let path = tape_path(&codec(&mut guard).pack);
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 3);
}

#[then(expr = "the wire-codec shard stops")]
fn shard_stops(w: &mut World) {
    drop_hold();
    w.pack = None;
}
