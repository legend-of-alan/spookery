use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, text_of, wait_has, write_stdin, Proc};
use crate::World;

// Procs and the GM socket cannot live on World. One scenario runs at a time.
struct Session {
    server: Proc,
    player: Proc,
    gm: Option<TcpStream>,
    next_id: u64,
    pack: PathBuf,
    stdout_mark: usize,
    stdout_commit: usize,
    commit_lines: Vec<String>,
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

fn oppose_pack(label: &str) -> PathBuf {
    let pack = support::temp_pack(label);
    let moves = pack.join("rules/moves.yaml");
    let mut text = fs::read_to_string(&moves).unwrap();
    text.push_str(OPPOSE_ROWS);
    fs::write(&moves, text).unwrap();
    fs::write(pack.join("cast/pc-nell.md"), NELL_CAST).unwrap();
    fs::write(pack.join("state/actors/pc_nell.yaml"), NELL_SHEET).unwrap();
    let now_path = pack.join("state/now.yaml");
    let mut now =
        spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.actors.insert(
        "pc_nell".to_string(),
        spookery_kernel::model::ActorLive::Pc {
            place: "docks_shop".to_string(),
            flags: Vec::new(),
            busy: false,
            known_facts: Vec::new(),
            sheet: "state/actors/pc_nell.yaml".to_string(),
        },
    );
    fs::write(&now_path, spookery_kernel::codec::emit_now(&now).unwrap()).unwrap();
    stamp_pack_rev(&pack);
    let canon = spookery_kernel::canon::load_canon(&pack).unwrap();
    let mark = canon.moves.get("mark").unwrap();
    assert_eq!(mark.check.as_deref(), Some("grit"));
    let mark_row = mark.row.as_ref().unwrap();
    assert_eq!(mark_row.token, spookery_kernel::moves::MoveToken::Actor);
    assert_eq!(mark_row.on, spookery_kernel::moves::MoveOn::Named);
    assert!(mark_row.effects.is_empty());
    let defend = canon.moves.get("defend").unwrap();
    assert_eq!(defend.check.as_deref(), Some("grit"));
    let defend_row = defend.row.as_ref().unwrap();
    assert_eq!(defend_row.token, spookery_kernel::moves::MoveToken::None);
    assert_eq!(defend_row.on, spookery_kernel::moves::MoveOn::Speaker);
    assert!(defend_row.effects.is_empty());
    assert_eq!(
        defend_row.success,
        vec![spookery_kernel::moves::RowEffect::Flag {
            key: "guarded".to_string(),
            value: true,
        }]
    );
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
    let session = guard.as_mut().expect("no oppose shard");
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

fn listen(w: &mut World) {
    drop_session();
    let pack = w.pack.clone().expect("oppose pack");
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let gm = Some(gm_session());
    let player = seat_player(&pack);
    *lock_session() = Some(Session {
        server,
        player,
        gm,
        next_id: 2,
        pack,
        stdout_mark: 0,
        stdout_commit: 0,
        commit_lines: Vec::new(),
    });
}

fn session_pack() -> PathBuf {
    lock_session()
        .as_ref()
        .expect("no oppose shard")
        .pack
        .clone()
}

fn gm_checks(lines: &[String]) -> Vec<Value> {
    let mut checks = Vec::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(params) = value.get("params") else {
            continue;
        };
        if params.get("kind").and_then(|kind| kind.as_str()) == Some("check") {
            checks.push(params.clone());
        }
    }
    checks
}

fn check_with_stat(checks: &[Value], stat: i64) -> Value {
    checks
        .iter()
        .find(|check| check["stat"].as_i64() == Some(stat))
        .unwrap_or_else(|| panic!("no stat {stat} in {checks:?}"))
        .clone()
}

fn assert_still_idle(pack: &Path) {
    assert_eq!(now_of(pack).world_time, 1080);
    assert!(tick_names(pack).is_empty());
}

fn assert_played(opposed: bool) {
    let (lines, pack) = {
        let guard = lock_session();
        let session = guard.as_ref().expect("no oppose shard");
        (session.commit_lines.clone(), session.pack.clone())
    };
    let checks = gm_checks(&lines);
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.world_time, 1085);
    assert_eq!(now.tick_index, 1);
    assert_eq!(rowan_place(&now), "docks_shop");
    assert_eq!(now.actors.get("pc_nell").unwrap().place(), "docks_shop");
    let tick = tick_value(&pack, "tick-0001.jsonl");
    assert_eq!(tick["world_time_before"], 1080);
    assert_eq!(tick["world_time_after"], 1085);
    let mark = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|intent| intent["move"] == "mark")
        .unwrap_or_else(|| panic!("no mark in {tick}"));
    assert_eq!(mark["actor_id"], "pc_rowan");
    assert_eq!(mark["source"], "player");
    assert_eq!(mark["target"]["actor"], "pc_nell");
    if opposed {
        assert_eq!(checks.len(), 2, "{checks:?}");
        let nell = check_with_stat(&checks, 100);
        let rowan = check_with_stat(&checks, 1);
        assert_eq!(nell["target"], 0);
        assert_eq!(nell["success"], true);
        let total = nell["total"].as_i64().unwrap();
        assert!(total >= 102, "{nell}");
        assert_eq!(rowan["target"], total);
        assert_eq!(rowan["success"], false);
        let defend = tick["intents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|intent| intent["move"] == "defend")
            .unwrap_or_else(|| panic!("no defend in {tick}"));
        assert_eq!(defend["actor_id"], "pc_nell");
        assert_eq!(defend["source"], "player");
        let mut flags = flags_of(&now, "pc_nell");
        flags.sort();
        assert_eq!(flags, vec!["guarded".to_string(), "missed".to_string()]);
    } else {
        assert_eq!(checks.len(), 1, "{checks:?}");
        assert_eq!(checks[0]["stat"], 1);
        assert_eq!(checks[0]["target"], 0);
        assert_eq!(checks[0]["success"], true);
        assert!(tick["intents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|intent| intent["move"] != "defend"));
        assert_eq!(flags_of(&now, "pc_nell"), vec!["marked".to_string()]);
    }
    assert!(flags_of(&now, "pc_rowan").is_empty());
    assert_no_derived_row(&tick, &["mark", "defend"]);
    drop_session();
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

const OPPOSE_ROWS: &str = "  - name: mark
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
  - name: defend
    token: none
    on: speaker
    check: grit
    success:
      - class: flag
        key: guarded
        value: true
";

const NELL_CAST: &str = "---
schema: cast/v0
id: pc_nell
kind: pc_template
facts: []
tick:
  eligible: false
  flags:
    - marked
    - missed
    - guarded
---
Nell keeps a guard up.
";

const NELL_SHEET: &str = "schema: sheet/v0
id: pc_nell
template: cast/pc-nell.md
stats:
  grit: 100
resources: {}
inventory: []
flags: []
";

#[given(expr = "the oppose port is locked")]
fn oppose_port_locked(w: &mut World) {
    drop_session();
    w.port = Some(crate::PortGuard(support::lock_port()));
}

#[given(expr = "an oppose pack labeled {string}")]
fn given_oppose_pack(w: &mut World, label: String) {
    w.pack = Some(oppose_pack(&label));
}

#[then(
    expr = "the oppose pack opens at state_rev 0 world_time 1080 tick_index 0 with Rowan and Nell in docks_shop and Nell has no flags"
)]
fn oppose_pack_opens(w: &mut World) {
    let pack = w.pack.as_ref().expect("oppose pack");
    let opened = now_of(pack);
    assert_eq!(opened.state_rev, 0);
    assert_eq!(opened.world_time, 1080);
    assert_eq!(opened.tick_index, 0);
    assert_eq!(rowan_place(&opened), "docks_shop");
    assert_eq!(opened.actors.get("pc_nell").unwrap().place(), "docks_shop");
    assert!(flags_of(&opened, "pc_nell").is_empty());
}

#[when(expr = "the oppose pack is running with a GM and a player")]
fn oppose_running(w: &mut World) {
    listen(w);
}

#[then(expr = "the oppose shard is still at world_time 1080 with no tick log")]
fn oppose_idle_after_seat(w: &mut World) {
    let _ = w;
    assert_still_idle(&session_pack());
}

#[when(expr = "the GM queues oppose defend for pc_nell")]
fn queue_defend(w: &mut World) {
    let _ = w;
    let defend = gm_tool(
        "intent",
        r#"{"actor_id":"pc_nell","actor_kind":"player","source":"player","move":"defend"}"#,
    );
    assert!(defend.get("code").is_none(), "{defend}");
    assert!(defend
        .get("queued")
        .and_then(|id| id.as_str())
        .is_some_and(|id| !id.is_empty()));
}

#[then(expr = "oppose defend is queued and the shard has not ticked")]
fn defend_queued_idle(w: &mut World) {
    let _ = w;
    assert_still_idle(&session_pack());
}

#[when(expr = "the player marks pc_nell on the oppose shard")]
fn player_marks(w: &mut World) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no oppose shard");
    session.stdout_mark = text_of(&session.player.stdout).len();
    write_stdin(&mut session.player, "mark pc_nell");
}

#[then(expr = "the oppose mark line is queued and the roll has not happened")]
fn mark_queued(w: &mut World) {
    let _ = w;
    let (stdout, before) = {
        let guard = lock_session();
        let session = guard.as_ref().expect("no oppose shard");
        (session.player.stdout.clone(), session.stdout_mark)
    };
    let queued_out = wait_has(&stdout, "\"kind\":\"queued\"", Duration::from_secs(5));
    let queued_added = &queued_out[before..];
    assert!(
        queued_added.contains("\"kind\":\"queued\""),
        "{queued_added}"
    );
    assert!(
        !queued_added.contains("\"kind\":\"check\""),
        "{queued_added}"
    );
    assert_still_idle(&session_pack());
}

#[then(expr = "oppose session status lists Rowan mark and no defend at world_time 1080")]
fn status_open(w: &mut World) {
    let _ = w;
    let status = gm_tool("session_status", "{}");
    let queued = status["queued"].as_array().unwrap();
    assert!(
        queued.iter().all(|item| item["move"] != "defend"),
        "{status}"
    );
    assert!(
        queued
            .iter()
            .any(|item| item["actor_id"] == "pc_rowan" && item["move"] == "mark"),
        "{status}"
    );
    assert_eq!(status["world_time"], 1080);
}

#[then(expr = "oppose session status lists Nell defend and Rowan mark at world_time 1080")]
fn status_held(w: &mut World) {
    let _ = w;
    let status = gm_tool("session_status", "{}");
    let queued = status["queued"].as_array().unwrap();
    assert!(
        queued
            .iter()
            .any(|item| { item["actor_id"] == "pc_nell" && item["move"] == "defend" }),
        "{status}"
    );
    assert!(
        queued
            .iter()
            .any(|item| item["actor_id"] == "pc_rowan" && item["move"] == "mark"),
        "{status}"
    );
    assert_eq!(status["world_time"], 1080);
}

#[when(expr = "the GM commits the oppose turn")]
fn commit_oppose(w: &mut World) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no oppose shard");
    session.stdout_commit = text_of(&session.player.stdout).len();
    let id = session.next_id;
    let lines = call(
        session.gm.as_mut().expect("no GM"),
        id,
        "commit_turn",
        r#"{"include_tick":true}"#,
    );
    session.next_id += 1;
    let committed = structured(result_line(&lines, id));
    assert!(committed.get("code").is_none(), "{committed}");
    session.commit_lines = lines;
}

#[then(expr = "the player sees oppose check success {word} and no faces")]
fn sees_check(w: &mut World, word: String) {
    let _ = w;
    let line = format!("{{\"kind\":\"check\",\"success\":{word}}}");
    let (stdout, stderr, before) = {
        let guard = lock_session();
        let session = guard.as_ref().expect("no oppose shard");
        (
            session.player.stdout.clone(),
            session.player.stderr.clone(),
            session.stdout_commit,
        )
    };
    let text = wait_has(&stdout, &line, Duration::from_secs(5));
    let added = &text[before..];
    assert!(added.contains(&line), "{added}");
    assert!(!added.contains("\"faces\""), "{added}");
    assert!(text_of(&stderr).is_empty(), "{}", text_of(&stderr));
}

#[then(expr = "the open oppose tick resolved one check and marked Nell")]
fn open_tick(w: &mut World) {
    let _ = w;
    assert_played(false);
}

#[then(expr = "the held oppose tick resolved both checks and missed Nell")]
fn held_tick(w: &mut World) {
    let _ = w;
    assert_played(true);
}
