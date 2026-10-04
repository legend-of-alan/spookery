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
    stderr_before: String,
    queued_seen: usize,
    resume_seen: usize,
    pack: PathBuf,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

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
        .expect("no flag shard")
        .pack
        .clone()
}

fn current_now() -> spookery_kernel::model::Now {
    now_of(&current_pack())
}

fn now_of(pack: &Path) -> spookery_kernel::model::Now {
    spookery_kernel::open_shard(pack)
        .unwrap_or_else(|err| panic!("open: {err}"))
        .now()
        .clone()
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

fn saved_id(w: &World, slot: &str) -> String {
    w.scratch
        .get(slot)
        .cloned()
        .unwrap_or_else(|| panic!("missing {slot} id"))
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

fn gm_tool(name: &str, arguments: &str) -> Value {
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no flag shard");
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

fn listen(w: &mut World, with_gm: bool) {
    drop_session();
    let pack = w.pack.clone().expect("flag pack");
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let gm = if with_gm { Some(gm_session()) } else { None };
    let player = seat_player(&pack);
    *lock_session() = Some(Session {
        server,
        player,
        gm,
        next_id: 2,
        schema_seen: 0,
        spec_seen: 0,
        stdout_before: String::new(),
        stderr_before: String::new(),
        queued_seen: 0,
        resume_seen: 0,
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

#[given(expr = "the flag queue port is locked")]
fn flag_port_locked(w: &mut World) {
    drop_session();
    w.port = Some(crate::PortGuard(support::lock_port()));
}

#[given(expr = "a flag action pack labeled {string}")]
fn flag_action_pack(w: &mut World, label: String) {
    w.pack = Some(action_pack(&label));
}

#[when(expr = "a flag player sits with a GM")]
fn flag_sits_with_gm(w: &mut World) {
    listen(w, true);
}

#[when(expr = "a flag player sits with no GM")]
fn flag_sits_without_gm(w: &mut World) {
    listen(w, false);
}

#[when(expr = "the player line {string} is flag {word}")]
fn flag_refuse(w: &mut World, line: String, code: String) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no flag shard");
    let stdout = text_of(&session.player.stdout);
    write_stdin(&mut session.player, &line);
    let wanted = match code.as_str() {
        "SCHEMA" => {
            session.schema_seen += 1;
            session.schema_seen
        }
        "NOT_IN_SPEC" => {
            session.spec_seen += 1;
            session.spec_seen
        }
        other => panic!("unexpected code {other}"),
    };
    let stderr = wait_count(
        &session.player.stderr,
        &code,
        wanted,
        Duration::from_secs(5),
    );
    assert_eq!(stderr.matches(code.as_str()).count(), wanted, "{line}\n{stderr}");
    assert_eq!(
        text_of(&session.player.stdout),
        stdout,
        "{line} wrote on stdout"
    );
}

#[when(expr = "the player writes the flag line {string}")]
fn flag_write(w: &mut World, line: String) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no flag shard");
    let stdout = text_of(&session.player.stdout);
    session.stdout_before = stdout.clone();
    session.stderr_before = text_of(&session.player.stderr);
    session.queued_seen = stdout.matches("\"kind\":\"queued\"").count();
    session.resume_seen = stdout.matches("\"kind\":\"resume\"").count();
    write_stdin(&mut session.player, &line);
}

#[when(expr = "the GM flag-commits the turn")]
fn flag_commit(w: &mut World) {
    let body = gm_tool("commit_turn", r#"{"include_tick":true}"#);
    w.scratch
        .insert("commit".to_string(), body.to_string());
}

#[then(expr = "flag humans is {int}")]
fn flag_humans(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().presence.humans as i64, n);
}

#[then(expr = "flag world_time is {int}")]
fn flag_world_time(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().world_time, n as u64);
}

#[then(expr = "flag world_time is not {int}")]
fn flag_world_time_is_not(w: &mut World, n: i64) {
    let _ = w;
    assert_ne!(current_now().world_time, n as u64);
}

#[then(expr = "flag tick_index is {int}")]
fn flag_tick_index(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().tick_index, n as u64);
}

#[then(expr = "flag Rowan is in {word}")]
fn flag_rowan(w: &mut World, place: String) {
    let _ = w;
    assert_eq!(rowan_place(&current_now()), place);
}

#[then(expr = "flag actor {word} has no flags")]
fn flag_actor_empty(w: &mut World, id: String) {
    let _ = w;
    assert!(flags_of(&current_now(), &id).is_empty(), "{id}");
}

#[then(expr = "flag actor {word} flags are {word}")]
fn flag_actor_flags(w: &mut World, id: String, flag: String) {
    let _ = w;
    assert_eq!(flags_of(&current_now(), &id), vec![flag]);
}

#[then(expr = "flag actor {word} is absent")]
fn flag_actor_absent(w: &mut World, id: String) {
    let _ = w;
    assert!(!current_now().actors.contains_key(&id), "{id}");
}

#[then(expr = "flag tick files are none")]
fn flag_no_ticks(w: &mut World) {
    let _ = w;
    assert!(tick_names(&current_pack()).is_empty());
}

#[then(expr = "flag GM queue is empty")]
fn flag_queue_empty(w: &mut World) {
    let _ = w;
    let status = gm_tool("session_status", "{}");
    assert!(queued_rows(&status).is_empty(), "{status}");
}

#[then(expr = "flag GM queue is the saved {word} id for move {word}")]
fn flag_queue_is(w: &mut World, slot: String, move_name: String) {
    let id = saved_id(w, &slot);
    let status = gm_tool("session_status", "{}");
    assert_eq!(queued_rows(&status), vec![(id, move_name)], "{status}");
}

#[then(expr = "the flag player gained one queued id and no resume")]
fn flag_gained_mark(w: &mut World) {
    let guard = lock_session();
    let session = guard.as_ref().expect("no flag shard");
    let stdout = wait_has(
        &session.player.stdout,
        "\"kind\":\"queued\"",
        Duration::from_secs(5),
    );
    let added = stdout[session.stdout_before.len()..].to_string();
    assert!(added.contains("\"kind\":\"queued\""), "{added}");
    assert!(!added.contains("\"kind\":\"resume\""), "{added}");
    assert_eq!(text_of(&session.player.stderr), session.stderr_before);
    let ids = queued_ids(&added);
    assert_eq!(ids.len(), 1, "{added}");
    drop(guard);
    w.scratch.insert("mark".to_string(), ids[0].clone());
}

#[then(expr = "the flag player gained one queued id saved as {word}")]
fn flag_gained_saved(w: &mut World, slot: String) {
    let (queued_seen, before_len) = {
        let guard = lock_session();
        let session = guard.as_ref().expect("no flag shard");
        (session.queued_seen, session.stdout_before.len())
    };
    let stdout = {
        let guard = lock_session();
        let session = guard.as_ref().expect("no flag shard");
        wait_count(
            &session.player.stdout,
            "\"kind\":\"queued\"",
            queued_seen + 1,
            Duration::from_secs(5),
        )
    };
    let added = stdout[before_len..].to_string();
    let ids = queued_ids(&added);
    assert_eq!(ids.len(), 1, "{added}");
    if slot == "missing" {
        assert_ne!(ids[0], saved_id(w, "mark"));
    }
    w.scratch.insert(slot, ids[0].clone());
}

#[then(expr = "the flag player gained a resume and no queued line")]
fn flag_gained_resume(w: &mut World) {
    let _ = w;
    let guard = lock_session();
    let session = guard.as_ref().expect("no flag shard");
    let stdout = wait_count(
        &session.player.stdout,
        "\"kind\":\"resume\"",
        session.resume_seen + 1,
        Duration::from_secs(5),
    );
    let added = &stdout[session.stdout_before.len()..];
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
    assert!(!added.contains("\"kind\":\"queued\""), "{added}");
}

#[then(expr = "the flag commit has no code")]
fn flag_commit_ok(w: &mut World) {
    let raw = w.scratch.get("commit").expect("no flag commit");
    let body: Value = serde_json::from_str(raw).unwrap();
    assert!(body.get("code").is_none(), "{body}");
}

#[then(expr = "flag tick {string} world_time goes from {int} to {int}")]
fn flag_tick_span(w: &mut World, name: String, before: i64, after: i64) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    assert_eq!(tick["world_time_before"], before);
    assert_eq!(tick["world_time_after"], after);
}

#[then(expr = "flag tick {string} has one player mark intent for the saved mark id targeting mira")]
fn flag_mark_intent(w: &mut World, name: String) {
    let mark_id = saved_id(w, "mark");
    let tick = tick_value(&current_pack(), &name);
    let player_marks: Vec<_> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "mark")
        .collect();
    assert_eq!(player_marks.len(), 1, "{tick}");
    assert_eq!(player_marks[0]["id"], mark_id);
    assert_eq!(player_marks[0]["source"], "player");
    assert_eq!(player_marks[0]["actor_id"], "pc_rowan");
    assert_eq!(player_marks[0]["target"]["actor"], "mira");
}

#[then(expr = "flag tick {string} has one flag effect for the saved mark id on mira key marked")]
fn flag_mark_effect(w: &mut World, name: String) {
    let mark_id = saved_id(w, "mark");
    let tick = tick_value(&current_pack(), &name);
    let flags = flag_effects(&tick);
    assert_eq!(flags.len(), 1, "{tick}");
    assert_eq!(flags[0]["as"], "player:pc_rowan");
    assert_eq!(flags[0]["intent_id"], mark_id);
    assert_eq!(flags[0]["actor_id"], "mira");
    assert_eq!(flags[0]["key"], "marked");
    assert_eq!(flags[0]["value"], true);
}

#[then(expr = "flag tick {string} has one flag effect on mira key marked")]
fn flag_live_effect(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    let flags = flag_effects(&tick);
    assert_eq!(flags.len(), 1, "{tick}");
    assert_eq!(flags[0]["actor_id"], "mira");
    assert_eq!(flags[0]["key"], "marked");
}

#[then(expr = "flag tick {string} derives neither mark nor wave")]
fn flag_no_derived(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    assert_no_derived_row(&tick, &["mark", "wave"]);
}

#[then(expr = "flag tick {string} rejects the saved {word} id as {word}")]
fn flag_rejects(w: &mut World, name: String, slot: String, reason: String) {
    let id = saved_id(w, &slot);
    let tick = tick_value(&current_pack(), &name);
    assert_eq!(
        tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == id)
            .unwrap()["reason"],
        reason
    );
}

#[then(expr = "flag tick {string} has no flag effects")]
fn flag_no_effects(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    assert!(flag_effects(&tick).is_empty(), "{tick}");
}

#[then(expr = "flag tick {string} intents omit the saved {word} id")]
fn flag_intents_omit(w: &mut World, name: String, slot: String) {
    let id = saved_id(w, &slot);
    let tick = tick_value(&current_pack(), &name);
    assert!(tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|intent| intent["id"] != id));
}

#[then(expr = "flag tick {string} closes effects for the saved {word} id")]
fn flag_closed(w: &mut World, name: String, slot: String) {
    let id = saved_id(w, &slot);
    let tick = tick_value(&current_pack(), &name);
    assert_closed_effects(&tick, &id);
}

#[then(expr = "the flag session is closed")]
fn flag_closed_session(w: &mut World) {
    let _ = w;
    drop_session();
}
