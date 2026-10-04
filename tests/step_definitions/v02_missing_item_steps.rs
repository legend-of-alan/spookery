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
    seen: usize,
    stdout_at: usize,
    pack: PathBuf,
    commit_lines: Vec<String>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

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

const ITEM_ROWS: &str = "  - name: mark
    audience: player
    token: actor
    on: named
    check: grit
    item: knife
    success:
      - class: flag
        key: marked
        value: true
    failure:
      - class: flag
        key: missed
        value: true
  - name: defend
    audience: player
    token: none
    on: speaker
    check: grit
    success:
      - class: flag
        key: guarded
        value: true
";

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
        .expect("no item shard")
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

fn assert_no_derived_row(tick: &Value, moves: &[&str]) {
    let tick_index = tick["tick_index"].as_u64().unwrap();
    let intents = tick["intents"].as_array().unwrap();
    if let Some(sample) = intents.iter().find(|intent| intent["source"] == "kernel") {
        let actor = sample["actor_id"].as_str().unwrap();
        let move_name = sample["move"].as_str().unwrap();
        let expect =
            spookery_kernel::ids::kernel_intent_id("docks-001", tick_index, actor, move_name);
        assert_eq!(sample["id"].as_str().unwrap(), expect, "{sample}");
    }
    for actor in ["mira", "bram", "aldric", "pc_rowan"] {
        for move_name in moves {
            let id = spookery_kernel::ids::kernel_intent_id(
                "docks-001",
                tick_index,
                actor,
                move_name,
            );
            assert!(
                intents.iter().all(|intent| {
                    intent["id"] != id || intent["source"] == "player"
                }),
                "{actor} derived {move_name}"
            );
            if actor != "pc_rowan" {
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
    }
    assert!(intents.iter().all(|intent| {
        intent["source"] != "kernel" || !moves.contains(&intent["move"].as_str().unwrap())
    }));
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

fn gm_call(name: &str, arguments: &str) -> (Value, Vec<String>) {
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no item shard");
    let id = session.next_id;
    let stream = session.gm.as_mut().expect("no GM");
    let lines = call(stream, id, name, arguments);
    let line = result_line(&lines, id);
    assert!(line.contains("\"result\""), "{name}: {line}");
    let body = structured(line);
    session.next_id += 1;
    (body, lines)
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

fn check_with_stat(checks: &[Value], stat: i64) -> &Value {
    checks
        .iter()
        .find(|check| check["stat"].as_i64() == Some(stat))
        .unwrap_or_else(|| panic!("no stat {stat} in {checks:?}"))
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

fn item_pack(label: &str, held: bool) -> PathBuf {
    let pack = support::temp_pack(label);
    let moves = pack.join("rules/moves.yaml");
    let mut text = fs::read_to_string(&moves).unwrap();
    text.push_str(ITEM_ROWS);
    fs::write(&moves, text).unwrap();
    fs::write(pack.join("cast/pc-nell.md"), NELL_CAST).unwrap();
    fs::write(pack.join("state/actors/pc_nell.yaml"), NELL_SHEET).unwrap();
    if held {
        let sheet = pack.join("state/actors/pc_rowan.yaml");
        let sheet_text = fs::read_to_string(&sheet).unwrap();
        let sheet_next = sheet_text.replacen("inventory: []\n", "inventory:\n  - knife\n", 1);
        assert_ne!(sheet_next, sheet_text, "rowan inventory was not patched");
        fs::write(&sheet, sheet_next).unwrap();
    }
    let now_path = pack.join("state/now.yaml");
    let mut now = spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
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
    pack
}

fn remember_sheet(w: &mut World) {
    let pack = w.pack.as_ref().expect("item pack");
    let sheet = fs::read_to_string(pack.join("state/actors/pc_rowan.yaml")).unwrap();
    w.scratch.insert("sheet_before".to_string(), sheet);
}

fn load_err(pack: &Path) -> spookery_kernel::Reason {
    spookery_kernel::canon::load_canon(pack).expect_err("pack load")
}

fn reject_item_moves(label: &str, row: &str) -> PathBuf {
    let pack = support::temp_pack(label);
    let path = pack.join("rules/moves.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let next = format!("{text}{row}");
    assert_ne!(next, text, "{label} did not change moves.yaml");
    fs::write(&path, next).unwrap();
    pack
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
    let pack = w.pack.clone().expect("item pack");
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
        seen: 0,
        stdout_at: 0,
        pack,
        commit_lines: Vec::new(),
    });
}

#[given(expr = "an item moves rewrite labeled {string} with an empty item id")]
fn item_empty_id(w: &mut World, label: String) {
    let row = "  - name: mark\n    audience: player\n    token: actor\n    on: named\n    item: \"\"\n    success:\n      - class: flag\n        key: marked\n        value: true\n";
    w.pack = Some(reject_item_moves(&label, row));
}

#[given(expr = "an item moves rewrite labeled {string} with an unknown gear key")]
fn item_unknown_gear(w: &mut World, label: String) {
    let row = "  - name: mark\n    audience: player\n    token: actor\n    on: named\n    item: knife\n    gear: extra\n    success:\n      - class: flag\n        key: marked\n        value: true\n";
    w.pack = Some(reject_item_moves(&label, row));
}

#[then(expr = "that item pack load is SCHEMA")]
fn item_load_schema(w: &mut World) {
    let pack = w.pack.as_ref().expect("item pack");
    assert_eq!(
        load_err(pack),
        spookery_kernel::Reason::Schema,
        "{}",
        pack.display()
    );
    let _ = fs::remove_dir_all(pack);
    w.pack = None;
}

#[given(expr = "the item port is locked")]
fn item_port_locked(w: &mut World) {
    drop_session();
    w.port = Some(crate::PortGuard(support::lock_port()));
}

#[given(expr = "an item pack labeled {string} where Rowan lacks knife")]
fn item_pack_lacks(w: &mut World, label: String) {
    w.pack = Some(item_pack(&label, false));
    remember_sheet(w);
}

#[given(expr = "an item pack labeled {string} where Rowan holds knife")]
fn item_pack_holds(w: &mut World, label: String) {
    w.pack = Some(item_pack(&label, true));
    remember_sheet(w);
}

#[then(expr = "the item mark row requires knife and defend names no item")]
fn item_rows(w: &mut World) {
    let pack = w.pack.as_ref().expect("item pack");
    let canon = spookery_kernel::canon::load_canon(pack).unwrap();
    let mark = canon.moves.get("mark").unwrap();
    assert_eq!(mark.check.as_deref(), Some("grit"));
    let mark_row = mark.row.as_ref().unwrap();
    assert_eq!(mark_row.item.as_deref(), Some("knife"));
    assert!(mark_row.effects.is_empty());
    let defend = canon.moves.get("defend").unwrap();
    let defend_row = defend.row.as_ref().unwrap();
    assert!(defend_row.item.is_none());
}

#[then(expr = "the item sheet lists an empty inventory")]
fn item_sheet_empty(w: &mut World) {
    let sheet = w.scratch.get("sheet_before").expect("sheet");
    assert!(sheet.contains("inventory: []\n"), "{sheet}");
}

#[then(expr = "the item sheet lists knife")]
fn item_sheet_knife(w: &mut World) {
    let sheet = w.scratch.get("sheet_before").expect("sheet");
    assert!(sheet.contains("\n  - knife\n"), "{sheet}");
}

#[when(expr = "an item player sits with a GM")]
fn item_sits_with_gm(w: &mut World) {
    listen(w, true);
}

#[when(expr = "an item player sits with no GM")]
fn item_sits_without_gm(w: &mut World) {
    listen(w, false);
}

#[then(expr = "the item player stderr is empty")]
fn item_stderr_empty(w: &mut World) {
    let _ = w;
    let guard = lock_session();
    let session = guard.as_ref().expect("no item shard");
    assert!(
        text_of(&session.player.stderr).is_empty(),
        "{}",
        text_of(&session.player.stderr)
    );
}

#[then(expr = "item humans is {int}")]
fn item_humans(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().presence.humans as i64, n);
}

#[then(expr = "item world_time is {int}")]
fn item_world_time(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().world_time, n as u64);
}

#[then(expr = "item tick_index is {int}")]
fn item_tick_index(w: &mut World, n: i64) {
    let _ = w;
    assert_eq!(current_now().tick_index, n as u64);
}

#[then(expr = "item tick files are none")]
fn item_no_ticks(w: &mut World) {
    let _ = w;
    assert!(tick_names(&current_pack()).is_empty());
}

#[then(expr = "item Rowan is in {word}")]
fn item_rowan(w: &mut World, place: String) {
    let _ = w;
    assert_eq!(rowan_place(&current_now()), place);
}

#[then(expr = "item actor {word} has no flags")]
fn item_actor_empty(w: &mut World, id: String) {
    let _ = w;
    assert!(flags_of(&current_now(), &id).is_empty(), "{id}");
}

#[then(expr = "item actor {word} is in {word}")]
fn item_actor_place(w: &mut World, id: String, place: String) {
    let _ = w;
    let now = current_now();
    assert_eq!(
        now.actors
            .get(&id)
            .unwrap_or_else(|| panic!("{id} missing"))
            .place(),
        place
    );
}

#[then(expr = "item actor {word} flags are guarded and missed")]
fn item_nell_flags(w: &mut World, id: String) {
    let _ = w;
    let mut flags = flags_of(&current_now(), &id);
    flags.sort();
    assert_eq!(flags, vec!["guarded".to_string(), "missed".to_string()]);
}

#[when(expr = "the player line {string} is item {word}")]
fn item_refuse(w: &mut World, line: String, code: String) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no item shard");
    let stdout = text_of(&session.player.stdout);
    write_stdin(&mut session.player, &line);
    session.seen += 1;
    let wanted = session.seen;
    let stderr = wait_count(&session.player.stderr, &code, wanted, Duration::from_secs(5));
    assert_eq!(
        stderr.matches(code.as_str()).count(),
        wanted,
        "{line}\n{stderr}"
    );
    assert_eq!(text_of(&session.player.stdout), stdout, "{line} wrote on stdout");
}

#[then(expr = "the item Rowan sheet is unchanged")]
fn item_sheet_unchanged(w: &mut World) {
    let before = w.scratch.get("sheet_before").expect("sheet").clone();
    let pack = w.pack.as_ref().expect("item pack");
    let sheet = fs::read_to_string(pack.join("state/actors/pc_rowan.yaml")).unwrap();
    assert_eq!(sheet, before);
}

#[then(expr = "item GM status queue is empty at world_time {int}")]
fn item_gm_queue_empty(w: &mut World, world_time: i64) {
    let _ = w;
    let (status, _) = gm_call("session_status", "{}");
    let queued = status["queued"].as_array().unwrap();
    assert!(queued.is_empty(), "{status}");
    assert_eq!(status["world_time"], world_time);
}

#[when(expr = "the GM queues defend for pc_nell")]
fn item_queue_defend(w: &mut World) {
    let (defend, _) = gm_call(
        "intent",
        r#"{"actor_id":"pc_nell","actor_kind":"player","source":"player","move":"defend"}"#,
    );
    w.scratch.insert("defend".to_string(), defend.to_string());
}

#[then(expr = "the defend intent has no code and a queued id")]
fn item_defend_queued(w: &mut World) {
    let defend: Value = serde_json::from_str(w.scratch.get("defend").expect("defend")).unwrap();
    assert!(defend.get("code").is_none(), "{defend}");
    assert!(defend
        .get("queued")
        .and_then(|id| id.as_str())
        .is_some_and(|id| !id.is_empty()));
}

#[when(expr = "the player writes the item line {string}")]
fn item_write(w: &mut World, line: String) {
    let _ = w;
    let mut guard = lock_session();
    let session = guard.as_mut().expect("no item shard");
    session.stdout_at = text_of(&session.player.stdout).len();
    write_stdin(&mut session.player, &line);
}

#[then(expr = "the item player gained queued and not check")]
fn item_gained_queued(w: &mut World) {
    let _ = w;
    let guard = lock_session();
    let session = guard.as_ref().expect("no item shard");
    let stdout = wait_has(
        &session.player.stdout,
        "\"kind\":\"queued\"",
        Duration::from_secs(5),
    );
    let added = &stdout[session.stdout_at..];
    assert!(added.contains("\"kind\":\"queued\""), "{added}");
    assert!(!added.contains("\"kind\":\"check\""), "{added}");
}

#[then(expr = "the item GM queue holds pc_nell defend and pc_rowan mark at world_time {int}")]
fn item_gm_queue_both(w: &mut World, world_time: i64) {
    let _ = w;
    let (status, _) = gm_call("session_status", "{}");
    let queued = status["queued"].as_array().unwrap();
    assert!(
        queued
            .iter()
            .any(|item| item["actor_id"] == "pc_nell" && item["move"] == "defend"),
        "{status}"
    );
    assert!(
        queued
            .iter()
            .any(|item| item["actor_id"] == "pc_rowan" && item["move"] == "mark"),
        "{status}"
    );
    assert_eq!(status["world_time"], world_time);
}

#[when(expr = "the GM item-commits the turn")]
fn item_commit(w: &mut World) {
    {
        let mut guard = lock_session();
        let session = guard.as_mut().expect("no item shard");
        session.stdout_at = text_of(&session.player.stdout).len();
    }
    let (committed, lines) = gm_call("commit_turn", r#"{"include_tick":true}"#);
    w.scratch
        .insert("commit".to_string(), committed.to_string());
    lock_session().as_mut().expect("no item shard").commit_lines = lines;
}

#[then(expr = "the item commit has no code")]
fn item_commit_ok(w: &mut World) {
    let committed: Value = serde_json::from_str(w.scratch.get("commit").expect("commit")).unwrap();
    assert!(committed.get("code").is_none(), "{committed}");
}

#[then(expr = "the item player gained a failed check and not faces")]
fn item_failed_check(w: &mut World) {
    let _ = w;
    let line = "{\"kind\":\"check\",\"success\":false}";
    let guard = lock_session();
    let session = guard.as_ref().expect("no item shard");
    let stdout = wait_has(&session.player.stdout, line, Duration::from_secs(5));
    let added = &stdout[session.stdout_at..];
    assert!(added.contains(line), "{added}");
    assert!(!added.contains("\"faces\""), "{added}");
}

#[then(expr = "the item GM checks are Nell success against Rowan")]
fn item_gm_checks(w: &mut World) {
    let _ = w;
    let guard = lock_session();
    let session = guard.as_ref().expect("no item shard");
    let checks = gm_checks(&session.commit_lines);
    assert_eq!(checks.len(), 2, "{checks:?}");
    let nell = check_with_stat(&checks, 100).clone();
    let rowan = check_with_stat(&checks, 1).clone();
    drop(guard);
    assert_eq!(nell["target"], 0);
    assert_eq!(nell["success"], true);
    let total = nell["total"].as_i64().unwrap();
    assert!(total >= 102, "{nell}");
    assert_eq!(rowan["target"], total);
    assert_eq!(rowan["success"], false);
}

#[then(expr = "item tick {string} world_time goes from {int} to {int}")]
fn item_tick_span(w: &mut World, name: String, before: i64, after: i64) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    assert_eq!(tick["world_time_before"], before);
    assert_eq!(tick["world_time_after"], after);
}

#[then(expr = "item tick {string} mark intent is player pc_rowan targeting pc_nell")]
fn item_mark_intent(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    let mark = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|intent| intent["move"] == "mark")
        .unwrap_or_else(|| panic!("no mark in {tick}"));
    assert_eq!(mark["actor_id"], "pc_rowan");
    assert_eq!(mark["source"], "player");
    assert_eq!(mark["target"]["actor"], "pc_nell");
}

#[then(expr = "item tick {string} defend intent is player pc_nell")]
fn item_defend_intent(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    let defend = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|intent| intent["move"] == "defend")
        .unwrap_or_else(|| panic!("no defend in {tick}"));
    assert_eq!(defend["actor_id"], "pc_nell");
    assert_eq!(defend["source"], "player");
}

#[then(expr = "item tick {string} derives neither mark nor defend")]
fn item_no_derived(w: &mut World, name: String) {
    let _ = w;
    let tick = tick_value(&current_pack(), &name);
    assert_no_derived_row(&tick, &["mark", "defend"]);
}

#[then(expr = "the item session is closed")]
fn item_closed(w: &mut World) {
    let _ = w;
    drop_session();
}
