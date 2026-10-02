use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use rmcp::model::CallToolRequestParam;
use rmcp::ServiceExt;
use serde_json::Value;

use crate::support::{self, Proc};
use crate::World;

// Procs and the GM socket cannot live on World. One scenario runs at a time.
// Fields drop in declaration order: the socket closes, then the server, then the port lock.
struct Hold {
    stream: Option<TcpStream>,
    server: Proc,
    #[allow(dead_code)]
    lock: support::PortLock,
    pack: PathBuf,
    before: Vec<u8>,
    rmcp: Option<RmcpReport>,
    raw: Option<RawReport>,
}

#[derive(Clone)]
struct RmcpReport {
    names: Vec<String>,
    codes: std::collections::BTreeMap<String, Option<String>>,
    intent: Value,
}

#[derive(Clone)]
struct RawReport {
    names: Vec<String>,
    descriptions: Vec<String>,
    player_lines: Vec<String>,
    player_line: String,
    player_body: Value,
    npc_body: Value,
    unscoped: Value,
    missing: Value,
    npc_act: Value,
    open: Value,
    patch_line: String,
    patch_body: Value,
    refusals: Vec<Refusal>,
}

#[derive(Clone)]
struct Refusal {
    name: String,
    line: String,
    body: Value,
}

const GM_TOOLS: &[&str] = &[
    "session_open",
    "session_status",
    "perceive",
    "intent",
    "say",
    "resolve_check",
    "apply_effect",
    "npc_act",
    "commit_turn",
    "recall",
    "session_save",
];

static HOLD: Mutex<Option<Hold>> = Mutex::new(None);

fn lock_hold() -> std::sync::MutexGuard<'static, Option<Hold>> {
    HOLD.lock().unwrap_or_else(|err| err.into_inner())
}

fn drop_hold() {
    let hold = lock_hold().take();
    let Some(mut hold) = hold else {
        return;
    };
    let pack = hold.pack.clone();
    let _ = hold.server.child.kill();
    let _ = hold.server.child.wait();
    drop(hold);
    let _ = fs::remove_dir_all(pack);
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

fn assert_tool_names(names: &[String]) {
    let got: Vec<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(got, GM_TOOLS);
}

fn pack_before() -> (PathBuf, Vec<u8>) {
    let hold = lock_hold();
    let hold = hold.as_ref().expect("no GM allowlist shard");
    (hold.pack.clone(), hold.before.clone())
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

#[given(expr = "a listening GM allowlist shard")]
fn listen(w: &mut World) {
    drop_hold();
    let lock = support::lock_port();
    let pack = support::temp_pack("tools");
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    w.pack = Some(pack.clone());
    *lock_hold() = Some(Hold {
        stream: None,
        server,
        lock,
        pack,
        before,
        rmcp: None,
        raw: None,
    });
}

fn empty_calls_report() -> RmcpReport {
    std::thread::spawn(|| {
        let runtime =
            tokio::runtime::Runtime::new().unwrap_or_else(|err| panic!("runtime: {err}"));
        runtime.block_on(empty_calls())
    })
    .join()
    .unwrap_or_else(|err| panic!("gm client panicked: {err:?}"))
}

async fn empty_calls() -> RmcpReport {
    use tokio::io::AsyncWriteExt;

    let mut stream = tokio::net::TcpStream::connect("127.0.0.1:9737")
        .await
        .unwrap_or_else(|err| panic!("connect: {err}"));
    stream.write_all(b"gm\n").await.unwrap();
    let running = ().serve(stream)
        .await
        .unwrap_or_else(|err| panic!("mcp: {err}"));
    let listed = tokio::time::timeout(Duration::from_secs(10), running.list_tools(None))
        .await
        .unwrap_or_else(|_| panic!("tools/list timed out"))
        .unwrap_or_else(|err| panic!("tools/list: {err}"));
    let names: Vec<String> = listed
        .tools
        .iter()
        .map(|tool| tool.name.as_ref().to_string())
        .collect();
    let mut codes = std::collections::BTreeMap::new();
    for name in [
        "session_open",
        "say",
        "perceive",
        "npc_act",
        "patch_entity",
        "commit_turn",
    ] {
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            running.call_tool(CallToolRequestParam {
                name: (*name).into(),
                arguments: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("{name} timed out"))
        .unwrap_or_else(|err| panic!("{name}: {err}"));
        let value = result
            .structured_content
            .unwrap_or_else(|| panic!("{name} has no object"));
        codes.insert(
            (*name).to_string(),
            value
                .get("code")
                .and_then(|code| code.as_str())
                .map(str::to_string),
        );
    }
    let intent = tokio::time::timeout(
        Duration::from_secs(10),
        running.call_tool(CallToolRequestParam {
            name: "intent".into(),
            arguments: Some(
                serde_json::json!({
                    "actor_id": "pc_rowan",
                    "actor_kind": "player",
                    "source": "player",
                    "move": "wait",
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("intent timed out"))
    .unwrap_or_else(|err| panic!("intent: {err}"));
    let intent = intent.structured_content.unwrap();
    RmcpReport {
        names,
        codes,
        intent,
    }
}

#[when(
    expr = "the GM lists tools and calls the empty-argument tools, commit_turn, and the pc_rowan wait intent"
)]
fn list_and_empty_calls(w: &mut World) {
    let _ = w;
    let report = empty_calls_report();
    lock_hold()
        .as_mut()
        .expect("no GM allowlist shard")
        .rmcp = Some(report);
}

fn rmcp_report() -> RmcpReport {
    lock_hold()
        .as_ref()
        .expect("no GM allowlist shard")
        .rmcp
        .clone()
        .expect("no empty-call report")
}

#[then(
    expr = "the listed GM tools are session_open, session_status, perceive, intent, say, resolve_check, apply_effect, npc_act, commit_turn, recall, session_save"
)]
fn listed_tools(w: &mut World) {
    let _ = w;
    assert_tool_names(&rmcp_report().names);
}

#[then(expr = "empty {word} returns {word}")]
fn empty_returns(w: &mut World, name: String, code: String) {
    let _ = w;
    let report = rmcp_report();
    assert_eq!(
        report.codes.get(&name).and_then(Option::as_deref),
        Some(code.as_str()),
        "{name}"
    );
}

#[then(expr = "the pc_rowan wait intent is queued and has no code")]
fn intent_queued(w: &mut World) {
    let _ = w;
    let intent = rmcp_report().intent;
    assert!(intent
        .get("queued")
        .and_then(|id| id.as_str())
        .is_some_and(|id| !id.is_empty()));
    assert!(intent.get("code").is_none());
}

#[then(
    expr = "Cargo.toml, src\\/kernel\\/Cargo.toml, src\\/mcp\\/Cargo.toml, and src\\/tick\\/Cargo.toml do not contain llm"
)]
fn no_llm_feature(w: &mut World) {
    let _ = w;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for path in [
        root.join("Cargo.toml"),
        root.join("src/kernel/Cargo.toml"),
        root.join("src/mcp/Cargo.toml"),
        root.join("src/tick/Cargo.toml"),
    ] {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        assert!(!text.contains("llm"), "{}", path.display());
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

fn raw_session_report(stream: &mut TcpStream) -> RawReport {
    write!(
        stream,
        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}}\n"
    )
    .unwrap();
    stream.flush().unwrap();
    let listed = read_lf(stream, Duration::from_secs(5));
    let listed_value: Value = serde_json::from_str(&listed).unwrap();
    let tools = listed_value["result"]["tools"].as_array().unwrap();
    let names: Vec<String> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    let descriptions: Vec<String> = tools
        .iter()
        .map(|tool| tool["description"].as_str().unwrap().to_string())
        .collect();

    let player_lines = call(stream, 3, "perceive", r#"{"as":"player:pc_rowan"}"#);
    let player_line = result_line(&player_lines, 3).to_string();
    let player_body = structured(&player_line);
    let npc_body = structured(result_line(
        &call(stream, 4, "perceive", r#"{"as":"npc:mira"}"#),
        4,
    ));
    let unscoped = structured(result_line(
        &call(stream, 5, "perceive", r#"{"as":"gm"}"#),
        5,
    ));
    let missing = structured(result_line(&call(stream, 6, "perceive", "{}"), 6));
    let npc_act = structured(result_line(
        &call(stream, 7, "npc_act", r#"{"actor_id":"mira"}"#),
        7,
    ));
    let open = structured(result_line(
        &call(
            stream,
            8,
            "session_open",
            r#"{"session_id":"s-gm","as":"player:pc_rowan"}"#,
        ),
        8,
    ));
    let patched = call(
        stream,
        9,
        "patch_entity",
        r#"{"path":"cast/mira.md","new_text":"rewritten"}"#,
    );
    let patch_line = result_line(&patched, 9).to_string();
    let patch_body = structured(&patch_line);
    let mut refusals = Vec::new();
    for name in [
        "resolve_check",
        "apply_effect",
        "recall",
        "session_save",
        "not_a_tool",
    ] {
        let lines = call(stream, 10, name, "{}");
        let line = result_line(&lines, 10).to_string();
        let body = structured(&line);
        refusals.push(Refusal {
            name: name.to_string(),
            line,
            body,
        });
    }
    RawReport {
        names,
        descriptions,
        player_lines,
        player_line,
        player_body,
        npc_body,
        unscoped,
        missing,
        npc_act,
        open,
        patch_line,
        patch_body,
        refusals,
    }
}

#[when(
    expr = "the GM raw session lists tools and calls perceive, npc_act, session_open, patch_entity, and the refusal names"
)]
fn raw_calls(w: &mut World) {
    let _ = w;
    let mut stream = gm_session();
    let report = raw_session_report(&mut stream);
    let mut hold = lock_hold();
    let hold = hold.as_mut().expect("no GM allowlist shard");
    hold.stream = Some(stream);
    hold.raw = Some(report);
}

fn raw_report() -> RawReport {
    lock_hold()
        .as_ref()
        .expect("no GM allowlist shard")
        .raw
        .clone()
        .expect("no raw GM report")
}

#[then(
    expr = "the raw listed GM tools are session_open, session_status, perceive, intent, say, resolve_check, apply_effect, npc_act, commit_turn, recall, session_save"
)]
fn raw_listed_tools(w: &mut World) {
    let _ = w;
    assert_tool_names(&raw_report().names);
}

#[then(expr = "every listed tool description contains {string}")]
fn description_contains(w: &mut World, phrase: String) {
    let _ = w;
    let raw = raw_report();
    assert_eq!(raw.descriptions.len(), GM_TOOLS.len());
    for description in &raw.descriptions {
        assert!(description.contains(&phrase), "{description}");
    }
}

#[then(
    expr = "perceive as player:pc_rowan is kind perceive with a place, no code, and no spook\\/event"
)]
fn player_perceive(w: &mut World) {
    let _ = w;
    let raw = raw_report();
    assert!(
        raw.player_line.contains(
            "\"structuredContent\":{\"kind\":\"perceive\",\"viewer\":\"player:pc_rowan\""
        ),
        "{}",
        raw.player_line
    );
    assert_eq!(raw.player_body["kind"], "perceive");
    assert_eq!(raw.player_body["viewer"], "player:pc_rowan");
    assert!(raw.player_body["place"].is_string(), "{}", raw.player_body);
    assert!(raw.player_body.get("code").is_none(), "{}", raw.player_body);
    assert!(
        raw.player_lines
            .iter()
            .all(|line| !line.contains("\"method\":\"spook/event\"")),
        "{:?}",
        raw.player_lines
    );
}

#[then(expr = "perceive as npc:mira is kind perceive for that viewer")]
fn npc_perceive(w: &mut World) {
    let _ = w;
    let raw = raw_report();
    assert_eq!(raw.npc_body["viewer"], "npc:mira");
    assert_eq!(raw.npc_body["kind"], "perceive");
}

#[then(expr = "perceive as gm returns UNSCOPED_PERCEIVE")]
fn unscoped_perceive(w: &mut World) {
    let _ = w;
    assert_eq!(raw_report().unscoped["code"], "UNSCOPED_PERCEIVE");
}

#[then(expr = "perceive with an empty object returns UNSCOPED_PERCEIVE")]
fn missing_perceive(w: &mut World) {
    let _ = w;
    assert_eq!(raw_report().missing["code"], "UNSCOPED_PERCEIVE");
}

#[then(expr = "npc_act for mira returns only LLM_DISABLED")]
fn npc_act_disabled(w: &mut World) {
    let _ = w;
    assert_eq!(
        raw_report().npc_act,
        serde_json::json!({ "code": "LLM_DISABLED" })
    );
}

#[then(expr = "session_open for s-gm as player:pc_rowan returns NOT_IN_SPEC")]
fn session_open_refused(w: &mut World) {
    let _ = w;
    assert_eq!(raw_report().open["code"], "NOT_IN_SPEC");
}

#[then(
    expr = "patch_entity of cast\\/mira.md returns NOT_IN_SPEC and not method-not-found"
)]
fn patch_refused(w: &mut World) {
    let _ = w;
    let raw = raw_report();
    assert!(raw.patch_line.contains("\"result\""), "{}", raw.patch_line);
    assert!(!raw.patch_line.contains("-32601"), "{}", raw.patch_line);
    assert_eq!(raw.patch_body["code"], "NOT_IN_SPEC");
}

#[then(
    expr = "resolve_check, apply_effect, recall, session_save, and not_a_tool return NOT_IN_SPEC and not method-not-found"
)]
fn other_refusals(w: &mut World) {
    let _ = w;
    let raw = raw_report();
    let names: Vec<&str> = raw.refusals.iter().map(|refusal| refusal.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "resolve_check",
            "apply_effect",
            "recall",
            "session_save",
            "not_a_tool",
        ]
    );
    for refusal in &raw.refusals {
        assert!(
            refusal.line.contains("\"result\""),
            "{}: {}",
            refusal.name,
            refusal.line
        );
        assert!(
            !refusal.line.contains("-32601"),
            "{}: {}",
            refusal.name,
            refusal.line
        );
        assert_eq!(refusal.body["code"], "NOT_IN_SPEC", "{}", refusal.name);
    }
}

#[then(expr = "the GM allowlist now.yaml is unchanged")]
fn now_unchanged(w: &mut World) {
    let _ = w;
    let (pack, before) = pack_before();
    assert_eq!(
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}")),
        before
    );
}

#[then(expr = "the GM allowlist pack has no tick file")]
fn no_tick_file(w: &mut World) {
    let _ = w;
    let (pack, _) = pack_before();
    assert!(tick_files(&pack).is_empty());
}

#[then(expr = "the GM allowlist pack has no session.jsonl")]
fn no_session_log(w: &mut World) {
    let _ = w;
    let (pack, _) = pack_before();
    assert!(!pack.join("state/log/session.jsonl").exists());
}

#[then(expr = "cast\\/mira.md does not contain rewritten")]
fn mira_not_rewritten(w: &mut World) {
    let _ = w;
    let (pack, _) = pack_before();
    let path = pack.join("cast/mira.md");
    assert!(
        !path.exists()
            || !fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
                .contains("rewritten")
    );
}

#[then(expr = "the GM allowlist shard stops")]
fn stop(w: &mut World) {
    drop_hold();
    w.pack = None;
}
