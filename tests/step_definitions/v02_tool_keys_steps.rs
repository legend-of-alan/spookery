use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cucumber::{then, when};
use serde_json::Value;

use crate::support::{self, Proc};
use crate::World;

// The GM socket and spook run cannot live on World. One scenario runs at a time.
// Fields drop in declaration order: the socket closes, then the server, then the port lock.
struct Hold {
    gm: TcpStream,
    server: Proc,
    #[allow(dead_code)]
    lock: support::PortLock,
    pack: PathBuf,
    before: Vec<u8>,
    tools: Vec<Value>,
    id: u64,
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

const PHRASES: &[(&str, &str)] = &[
    ("session_open", "returns NOT_IN_SPEC and writes nothing"),
    ("session_status", "empty object stays legal"),
    ("perceive", "A missing as is UNSCOPED_PERCEIVE"),
    ("intent", "source stays player"),
    ("say", "A supplied from is SCHEMA"),
    (
        "resolve_check",
        "Calling resolve_check on this seat returns NOT_IN_SPEC",
    ),
    (
        "apply_effect",
        "Calling apply_effect on this seat returns NOT_IN_SPEC",
    ),
    ("npc_act", "calls no model"),
    ("commit_turn", "requires include_tick"),
    ("recall", "Calling recall on this seat returns NOT_IN_SPEC"),
    (
        "session_save",
        "Calling session_save on this seat returns NOT_IN_SPEC",
    ),
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

fn refusal(stream: &mut TcpStream, id: u64, name: &str, arguments: &str) -> Value {
    let lines = call(stream, id, name, arguments);
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("\"method\":\"spook/event\"")),
        "{name}: {lines:?}"
    );
    let line = result_line(&lines, id);
    assert!(line.contains("\"result\""), "{name}: {line}");
    assert!(!line.contains("-32601"), "{name}: {line}");
    structured(line)
}

fn property_names(schema: &Value) -> BTreeSet<&str> {
    schema["properties"]
        .as_object()
        .map(|map| map.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

fn required_list(schema: &Value) -> Vec<&str> {
    schema["required"]
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn address_branch<'a>(schema: &'a Value, address: &str) -> &'a Value {
    schema["allOf"]
        .as_array()
        .unwrap_or_else(|| panic!("say schema has no allOf: {schema}"))
        .iter()
        .find(|branch| branch["if"]["properties"]["address"]["const"].as_str() == Some(address))
        .unwrap_or_else(|| panic!("no say branch for {address}: {schema}"))
}

fn tools() -> Vec<Value> {
    lock_hold()
        .as_ref()
        .expect("no tool-keys shard")
        .tools
        .clone()
}

fn schema_of(tools: &[Value], name: &str) -> Value {
    tools
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("missing {name}"))["inputSchema"]
        .clone()
}

fn with_gm(f: impl FnOnce(&mut TcpStream, &mut u64)) {
    let mut guard = lock_hold();
    let hold = guard.as_mut().expect("no tool-keys shard");
    f(&mut hold.gm, &mut hold.id);
}

fn next_id(id: &mut u64) -> u64 {
    let current = *id;
    *id += 1;
    current
}

#[when(expr = "the GM lists tools on the tool-keys shard")]
fn list_tools(w: &mut World) {
    drop_hold();
    let lock = support::lock_port();
    let pack = w.pack.clone().expect("temp pack");
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    let mut gm = gm_session();
    write!(
        gm,
        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}}\n"
    )
    .unwrap();
    gm.flush().unwrap();
    let listed = read_lf(&mut gm, Duration::from_secs(5));
    let listed_value: Value =
        serde_json::from_str(&listed).unwrap_or_else(|err| panic!("{err}: {listed}"));
    let tools = listed_value["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{listed}"))
        .clone();
    *lock_hold() = Some(Hold {
        gm,
        server,
        lock,
        pack,
        before,
        tools,
        id: 3,
    });
}

#[then(
    expr = "the tool-keys names are session_open, session_status, perceive, intent, say, resolve_check, apply_effect, npc_act, commit_turn, recall, session_save"
)]
fn names_are(_w: &mut World) {
    let names: Vec<String> = tools()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    let got: Vec<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(got, GM_TOOLS);
}

#[then(expr = "the tool-keys list does not include patch_entity")]
fn no_patch_entity(_w: &mut World) {
    let names: Vec<String> = tools()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    assert!(!names.iter().any(|name| name == "patch_entity"));
}

#[then(
    expr = "each tool-keys description contains its recorded phrase and the descriptions are distinct"
)]
fn descriptions_match(_w: &mut World) {
    let tools = tools();
    let mut descriptions = BTreeSet::new();
    for (name, phrase) in PHRASES {
        let tool = tools.iter().find(|tool| tool["name"] == *name).unwrap();
        let description = tool["description"].as_str().unwrap();
        assert!(description.contains(phrase), "{name}: {description}");
        assert!(
            descriptions.insert(description.to_string()),
            "duplicate description for {name}"
        );
    }
    assert_eq!(descriptions.len(), GM_TOOLS.len());
}

#[then(
    expr = "the tool-keys say schema requires address and text and branches table, place, and actor"
)]
fn say_schema(_w: &mut World) {
    let say = schema_of(&tools(), "say");
    assert_eq!(
        property_names(&say),
        BTreeSet::from(["address", "text", "place", "actor"])
    );
    assert_eq!(required_list(&say), ["address", "text"]);
    assert_eq!(say["additionalProperties"], false);
    assert_eq!(
        say["properties"]["address"]["enum"],
        serde_json::json!(["table", "place", "actor"])
    );
    let place = address_branch(&say, "place");
    assert_eq!(place["then"]["required"], serde_json::json!(["place"]));
    assert_eq!(place["then"]["properties"]["place"]["type"], "string");
    assert_eq!(place["then"]["properties"]["actor"]["type"], "null");
    let actor = address_branch(&say, "actor");
    assert_eq!(actor["then"]["required"], serde_json::json!(["actor"]));
    assert_eq!(actor["then"]["properties"]["actor"]["type"], "string");
    assert_eq!(actor["then"]["properties"]["place"]["type"], "null");
    let table = address_branch(&say, "table");
    assert!(table["then"].get("required").is_none(), "{table}");
    assert_eq!(table["then"]["properties"]["place"]["type"], "null");
    assert_eq!(table["then"]["properties"]["actor"]["type"], "null");
}

#[then(expr = "the tool-keys perceive schema requires as and accepts focus and budget_tokens")]
fn perceive_schema(_w: &mut World) {
    let perceive = schema_of(&tools(), "perceive");
    assert_eq!(
        property_names(&perceive),
        BTreeSet::from(["as", "focus", "budget_tokens"])
    );
    assert_eq!(required_list(&perceive), ["as"]);
    assert_eq!(perceive["additionalProperties"], false);
    assert_eq!(perceive["properties"]["as"]["type"], "string");
    assert_eq!(
        perceive["properties"]["focus"]["enum"],
        serde_json::json!(["place", "actor"])
    );
    assert_eq!(perceive["properties"]["budget_tokens"]["type"], "integer");
}

#[then(
    expr = "the tool-keys commit_turn schema requires include_tick and accepts session_id"
)]
fn commit_schema(_w: &mut World) {
    let commit = schema_of(&tools(), "commit_turn");
    assert_eq!(
        property_names(&commit),
        BTreeSet::from(["include_tick", "session_id"])
    );
    assert_eq!(required_list(&commit), ["include_tick"]);
    assert_eq!(commit["additionalProperties"], false);
    assert_eq!(commit["properties"]["include_tick"]["type"], "boolean");
    assert_eq!(commit["properties"]["session_id"]["type"], "string");
}

#[then(expr = "the tool-keys session_status schema accepts since and requires nothing")]
fn status_schema(_w: &mut World) {
    let status = schema_of(&tools(), "session_status");
    assert_eq!(property_names(&status), BTreeSet::from(["since"]));
    assert!(status.get("required").is_none(), "{status}");
    assert_eq!(status["additionalProperties"], false);
    assert_eq!(status["properties"]["since"]["type"], "integer");
}

#[then(
    expr = "the tool-keys intent schema requires actor_id, actor_kind, source, and move and source stays player"
)]
fn intent_schema(_w: &mut World) {
    let intent = schema_of(&tools(), "intent");
    assert_eq!(
        property_names(&intent),
        BTreeSet::from([
            "actor_id",
            "actor_kind",
            "source",
            "move",
            "id",
            "tick_index",
            "target",
            "args",
            "rationale_code",
            "llm_say",
        ])
    );
    assert_eq!(
        required_list(&intent),
        ["actor_id", "actor_kind", "source", "move"]
    );
    assert_eq!(intent["additionalProperties"], false);
    assert_eq!(intent["properties"]["source"]["const"], "player");
    assert_eq!(
        intent["properties"]["actor_kind"]["enum"],
        serde_json::json!(["npc", "player"])
    );
}

#[then(
    expr = "the tool-keys live schemas for say, perceive, commit_turn, session_status, and intent differ"
)]
fn live_schemas_differ(_w: &mut World) {
    let tools = tools();
    let live = ["say", "perceive", "commit_turn", "session_status", "intent"];
    for left in 0..live.len() {
        for right in (left + 1)..live.len() {
            let left_schema = schema_of(&tools, live[left]);
            let right_schema = schema_of(&tools, live[right]);
            assert_ne!(
                left_schema, right_schema,
                "{} and {}",
                live[left], live[right]
            );
        }
    }
}

#[then(
    expr = "the tool-keys schemas for session_open, resolve_check, apply_effect, npc_act, recall, and session_save are type object"
)]
fn refusal_schemas(_w: &mut World) {
    let tools = tools();
    for name in [
        "session_open",
        "resolve_check",
        "apply_effect",
        "npc_act",
        "recall",
        "session_save",
    ] {
        let schema = schema_of(&tools, name);
        assert_eq!(schema, serde_json::json!({ "type": "object" }), "{name}");
    }
}

#[then(expr = "tool-keys say returns SCHEMA when a required key is missing or from is supplied")]
fn say_refusals(_w: &mut World) {
    with_gm(|gm, id| {
        assert_eq!(refusal(gm, next_id(id), "say", "{}")["code"], "SCHEMA");
        assert_eq!(
            refusal(gm, next_id(id), "say", r#"{"text":"hi"}"#)["code"],
            "SCHEMA"
        );
        assert_eq!(
            refusal(gm, next_id(id), "say", r#"{"address":"table"}"#)["code"],
            "SCHEMA"
        );
        assert_eq!(
            refusal(
                gm,
                next_id(id),
                "say",
                r#"{"address":"place","text":"hi"}"#
            )["code"],
            "SCHEMA"
        );
        assert_eq!(
            refusal(
                gm,
                next_id(id),
                "say",
                r#"{"address":"actor","text":"hi"}"#
            )["code"],
            "SCHEMA"
        );
        assert_eq!(
            refusal(
                gm,
                next_id(id),
                "say",
                r#"{"address":"table","text":"hi","from":"gm"}"#
            )["code"],
            "SCHEMA"
        );
    });
}

#[then(expr = "tool-keys perceive returns UNSCOPED_PERCEIVE when as is missing")]
fn perceive_refusals(_w: &mut World) {
    with_gm(|gm, id| {
        assert_eq!(
            refusal(gm, next_id(id), "perceive", "{}")["code"],
            "UNSCOPED_PERCEIVE"
        );
        assert_eq!(
            refusal(gm, next_id(id), "perceive", r#"{"focus":"place"}"#)["code"],
            "UNSCOPED_PERCEIVE"
        );
    });
}

#[then(expr = "tool-keys commit_turn returns SCHEMA when include_tick is missing")]
fn commit_refusals(_w: &mut World) {
    with_gm(|gm, id| {
        assert_eq!(
            refusal(gm, next_id(id), "commit_turn", "{}")["code"],
            "SCHEMA"
        );
        assert_eq!(
            refusal(gm, next_id(id), "commit_turn", r#"{"session_id":"s"}"#)["code"],
            "SCHEMA"
        );
    });
}

#[then(
    expr = "tool-keys intent returns SCHEMA when actor_id, actor_kind, source, or move is missing"
)]
fn intent_refusals(_w: &mut World) {
    with_gm(|gm, id| {
        for missing in ["actor_id", "actor_kind", "source", "move"] {
            let mut args = serde_json::json!({
                "actor_id": "pc_rowan",
                "actor_kind": "player",
                "source": "player",
                "move": "wait",
            });
            args.as_object_mut().unwrap().remove(missing);
            assert_eq!(
                refusal(gm, next_id(id), "intent", &args.to_string())["code"],
                "SCHEMA",
                "{missing}"
            );
        }
    });
}

#[then(
    expr = "tool-keys session_status with an empty object has queued and last_seq and no code and no events"
)]
fn status_empty(_w: &mut World) {
    with_gm(|gm, id| {
        let status = refusal(gm, next_id(id), "session_status", "{}");
        assert!(status.get("code").is_none(), "{status}");
        assert!(status.get("queued").is_some(), "{status}");
        assert!(status.get("last_seq").is_some(), "{status}");
        assert!(status.get("events").is_none(), "{status}");
    });
}

#[then(expr = "tool-keys session_status with an extra key returns SCHEMA")]
fn status_extra(_w: &mut World) {
    with_gm(|gm, id| {
        assert_eq!(
            refusal(
                gm,
                next_id(id),
                "session_status",
                r#"{"since":0,"extra":true}"#
            )["code"],
            "SCHEMA"
        );
    });
}

#[then(expr = "tool-keys npc_act returns LLM_DISABLED")]
fn npc_act_disabled(_w: &mut World) {
    with_gm(|gm, id| {
        assert_eq!(
            refusal(gm, next_id(id), "npc_act", "{}")["code"],
            "LLM_DISABLED"
        );
    });
}

#[then(
    expr = "tool-keys session_open, patch_entity, resolve_check, apply_effect, recall, session_save, and not_a_tool return NOT_IN_SPEC"
)]
fn named_refusals(_w: &mut World) {
    with_gm(|gm, id| {
        for name in [
            "session_open",
            "patch_entity",
            "resolve_check",
            "apply_effect",
            "recall",
            "session_save",
            "not_a_tool",
        ] {
            assert_eq!(
                refusal(gm, next_id(id), name, "{}")["code"],
                "NOT_IN_SPEC",
                "{name}"
            );
        }
    });
}

#[then(expr = "the tool-keys now.yaml is unchanged and session.jsonl is absent")]
fn now_unchanged(_w: &mut World) {
    let guard = lock_hold();
    let hold = guard.as_ref().expect("no tool-keys shard");
    let now = fs::read(hold.pack.join("state/now.yaml")).unwrap();
    assert_eq!(now, hold.before);
    assert!(!hold.pack.join("state/log/session.jsonl").exists());
}

#[then(expr = "the tool-keys shard stops")]
fn stop(w: &mut World) {
    drop_hold();
    w.pack = None;
}
