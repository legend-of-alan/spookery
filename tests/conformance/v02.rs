use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::model::{CallToolRequestParam, ErrorCode};
use rmcp::transport::TokioChildProcess;
use rmcp::{ServiceError, ServiceExt};
use serde_json::Value;
use tokio::process::Command as TokioCommand;

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

const TABLE_TOOLS: &[&str] = &[
    "session_open",
    "session_status",
    "perceive",
    "intent",
    "resolve_check",
    "apply_effect",
    "npc_act",
    "commit_turn",
    "recall",
    "session_save",
];

struct PortLock(std::fs::File);

impl Drop for PortLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn lock_port() -> PortLock {
    let path = std::env::temp_dir().join("spookery-9737.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .unwrap_or_else(|err| panic!("lock {path:?}: {err}"));
    file.lock().unwrap_or_else(|err| panic!("lock {path:?}: {err}"));
    PortLock(file)
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())) {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest).unwrap_or_else(|err| panic!("copy {}: {err}", entry.path().display()));
        }
    }
}

fn temp_pack(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-v02-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&manifest.join("packs/oneshot-docks"), &pack);
    pack
}

struct Proc {
    child: Child,
    // Held open so the shard does not read EOF on stdin.
    #[allow(dead_code)]
    stdin: Option<ChildStdin>,
    #[allow(dead_code)]
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(args: &[&str]) -> Proc {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(args);
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap_or_else(|err| panic!("spawn spook: {err}"));
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    drain(stdout, Arc::clone(&stdout_buf));
    drain(stderr, Arc::clone(&stderr_buf));
    Proc {
        child,
        stdin,
        stdout: stdout_buf,
        stderr: stderr_buf,
    }
}

fn drain(reader: impl Read + Send + 'static, slot: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => slot.lock().unwrap().push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });
}

fn text_of(slot: &Arc<Mutex<String>>) -> String {
    slot.lock().unwrap().clone()
}

fn wait_listen(proc: &mut Proc) {
    let start = Instant::now();
    loop {
        if TcpStream::connect("127.0.0.1:9737").is_ok() {
            return;
        }
        if let Some(status) = proc.child.try_wait().unwrap() {
            panic!("spook run exited {:?}\n{}", status.code(), text_of(&proc.stderr));
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("listener did not open\n{}", text_of(&proc.stderr));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn pack_arg(pack: &Path) -> String {
    pack.to_str().unwrap().to_string()
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
    stream.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
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
        lines.iter().all(|line| !line.contains("\"method\":\"spook/event\"")),
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

#[test]
fn v02_p01_gm_tools_list_names_distinct_keys_and_keeps_the_refusals() {
    let _lock = lock_port();
    let pack = temp_pack("list");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    let mut gm = gm_session();
    write!(gm, "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}}\n").unwrap();
    gm.flush().unwrap();
    let listed = read_lf(&mut gm, Duration::from_secs(5));
    let listed_value: Value = serde_json::from_str(&listed).unwrap_or_else(|err| panic!("{err}: {listed}"));
    let tools = listed_value["result"]["tools"].as_array().unwrap_or_else(|| panic!("{listed}"));
    let names: Vec<&str> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
    assert_eq!(names, GM_TOOLS);
    assert!(!names.contains(&"patch_entity"));

    let mut descriptions = BTreeSet::new();
    for (name, phrase) in [
        ("session_open", "returns NOT_IN_SPEC and writes nothing"),
        ("session_status", "empty object stays legal"),
        ("perceive", "A missing as is UNSCOPED_PERCEIVE"),
        ("intent", "source stays player"),
        ("say", "A supplied from is SCHEMA"),
        ("resolve_check", "Calling resolve_check on this seat returns NOT_IN_SPEC"),
        ("apply_effect", "Calling apply_effect on this seat returns NOT_IN_SPEC"),
        ("npc_act", "calls no model"),
        ("commit_turn", "requires include_tick"),
        ("recall", "Calling recall on this seat returns NOT_IN_SPEC"),
        ("session_save", "Calling session_save on this seat returns NOT_IN_SPEC"),
    ] {
        let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
        let description = tool["description"].as_str().unwrap();
        assert!(description.contains(phrase), "{name}: {description}");
        assert!(descriptions.insert(description), "duplicate description for {name}");
    }
    assert_eq!(descriptions.len(), GM_TOOLS.len());

    let say = &tools.iter().find(|tool| tool["name"] == "say").unwrap()["inputSchema"];
    assert_eq!(
        property_names(say),
        BTreeSet::from(["address", "text", "place", "actor"])
    );
    assert_eq!(required_list(say), ["address", "text"]);
    assert_eq!(say["additionalProperties"], false);
    assert_eq!(say["properties"]["address"]["enum"], serde_json::json!(["table", "place", "actor"]));
    let place = address_branch(say, "place");
    assert_eq!(place["then"]["required"], serde_json::json!(["place"]));
    assert_eq!(place["then"]["properties"]["place"]["type"], "string");
    assert_eq!(place["then"]["properties"]["actor"]["type"], "null");
    let actor = address_branch(say, "actor");
    assert_eq!(actor["then"]["required"], serde_json::json!(["actor"]));
    assert_eq!(actor["then"]["properties"]["actor"]["type"], "string");
    assert_eq!(actor["then"]["properties"]["place"]["type"], "null");
    let table = address_branch(say, "table");
    assert!(table["then"].get("required").is_none(), "{table}");
    assert_eq!(table["then"]["properties"]["place"]["type"], "null");
    assert_eq!(table["then"]["properties"]["actor"]["type"], "null");

    let perceive = &tools.iter().find(|tool| tool["name"] == "perceive").unwrap()["inputSchema"];
    assert_eq!(property_names(perceive), BTreeSet::from(["as", "focus", "budget_tokens"]));
    assert_eq!(required_list(perceive), ["as"]);
    assert_eq!(perceive["additionalProperties"], false);
    assert_eq!(perceive["properties"]["as"]["type"], "string");
    assert_eq!(perceive["properties"]["focus"]["enum"], serde_json::json!(["place", "actor"]));
    assert_eq!(perceive["properties"]["budget_tokens"]["type"], "integer");

    let commit = &tools.iter().find(|tool| tool["name"] == "commit_turn").unwrap()["inputSchema"];
    assert_eq!(property_names(commit), BTreeSet::from(["include_tick", "session_id"]));
    assert_eq!(required_list(commit), ["include_tick"]);
    assert_eq!(commit["additionalProperties"], false);
    assert_eq!(commit["properties"]["include_tick"]["type"], "boolean");
    assert_eq!(commit["properties"]["session_id"]["type"], "string");

    let status = &tools.iter().find(|tool| tool["name"] == "session_status").unwrap()["inputSchema"];
    assert_eq!(property_names(status), BTreeSet::from(["since"]));
    assert!(status.get("required").is_none(), "{status}");
    assert_eq!(status["additionalProperties"], false);
    assert_eq!(status["properties"]["since"]["type"], "integer");

    let intent = &tools.iter().find(|tool| tool["name"] == "intent").unwrap()["inputSchema"];
    assert_eq!(
        property_names(intent),
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
    assert_eq!(required_list(intent), ["actor_id", "actor_kind", "source", "move"]);
    assert_eq!(intent["additionalProperties"], false);
    assert_eq!(intent["properties"]["source"]["const"], "player");
    assert_eq!(intent["properties"]["actor_kind"]["enum"], serde_json::json!(["npc", "player"]));

    let live = ["say", "perceive", "commit_turn", "session_status", "intent"];
    for left in 0..live.len() {
        for right in (left + 1)..live.len() {
            let left_schema = &tools.iter().find(|tool| tool["name"] == live[left]).unwrap()["inputSchema"];
            let right_schema = &tools.iter().find(|tool| tool["name"] == live[right]).unwrap()["inputSchema"];
            assert_ne!(left_schema, right_schema, "{} and {}", live[left], live[right]);
        }
    }
    for name in ["session_open", "resolve_check", "apply_effect", "npc_act", "recall", "session_save"] {
        let schema = &tools.iter().find(|tool| tool["name"] == name).unwrap()["inputSchema"];
        assert_eq!(schema, &serde_json::json!({ "type": "object" }), "{name}");
    }

    let mut id = 3u64;
    let mut next = || {
        let current = id;
        id += 1;
        current
    };
    assert_eq!(refusal(&mut gm, next(), "say", "{}")["code"], "SCHEMA");
    assert_eq!(refusal(&mut gm, next(), "say", r#"{"text":"hi"}"#)["code"], "SCHEMA");
    assert_eq!(refusal(&mut gm, next(), "say", r#"{"address":"table"}"#)["code"], "SCHEMA");
    assert_eq!(
        refusal(&mut gm, next(), "say", r#"{"address":"place","text":"hi"}"#)["code"],
        "SCHEMA"
    );
    assert_eq!(
        refusal(&mut gm, next(), "say", r#"{"address":"actor","text":"hi"}"#)["code"],
        "SCHEMA"
    );
    assert_eq!(
        refusal(&mut gm, next(), "say", r#"{"address":"table","text":"hi","from":"gm"}"#)["code"],
        "SCHEMA"
    );
    assert_eq!(refusal(&mut gm, next(), "perceive", "{}")["code"], "UNSCOPED_PERCEIVE");
    assert_eq!(
        refusal(&mut gm, next(), "perceive", r#"{"focus":"place"}"#)["code"],
        "UNSCOPED_PERCEIVE"
    );
    assert_eq!(refusal(&mut gm, next(), "commit_turn", "{}")["code"], "SCHEMA");
    assert_eq!(
        refusal(&mut gm, next(), "commit_turn", r#"{"session_id":"s"}"#)["code"],
        "SCHEMA"
    );
    for missing in ["actor_id", "actor_kind", "source", "move"] {
        let mut args = serde_json::json!({
            "actor_id": "pc_rowan",
            "actor_kind": "player",
            "source": "player",
            "move": "wait",
        });
        args.as_object_mut().unwrap().remove(missing);
        assert_eq!(refusal(&mut gm, next(), "intent", &args.to_string())["code"], "SCHEMA", "{missing}");
    }
    let status = refusal(&mut gm, next(), "session_status", "{}");
    assert!(status.get("code").is_none(), "{status}");
    assert!(status.get("queued").is_some(), "{status}");
    assert!(status.get("last_seq").is_some(), "{status}");
    assert!(status.get("events").is_none(), "{status}");
    assert_eq!(
        refusal(&mut gm, next(), "session_status", r#"{"since":0,"extra":true}"#)["code"],
        "SCHEMA"
    );
    assert_eq!(refusal(&mut gm, next(), "npc_act", "{}")["code"], "LLM_DISABLED");
    for name in [
        "session_open",
        "patch_entity",
        "resolve_check",
        "apply_effect",
        "recall",
        "session_save",
        "not_a_tool",
    ] {
        assert_eq!(refusal(&mut gm, next(), name, "{}")["code"], "NOT_IN_SPEC", "{name}");
    }

    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}

#[tokio::test]
async fn v02_p02_pack_table_stdio_does_not_gain_say_or_a_tools_call_body() {
    let pack = temp_pack("stdio");
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    let mut command = TokioCommand::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["run", pack.to_str().unwrap(), "--mcp", "pack-table"]);
    let running = ().serve(TokioChildProcess::new(command).unwrap_or_else(|err| panic!("spawn spook: {err}")));
    let running = tokio::time::timeout(Duration::from_secs(15), running)
        .await
        .unwrap_or_else(|_| panic!("initialize timed out"))
        .unwrap_or_else(|err| panic!("initialize: {err}"));
    let listed = tokio::time::timeout(Duration::from_secs(15), running.list_tools(None))
        .await
        .unwrap_or_else(|_| panic!("tools/list timed out"))
        .unwrap_or_else(|err| panic!("tools/list: {err}"));
    let names: Vec<&str> = listed.tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, TABLE_TOOLS);
    assert!(!names.contains(&"say"));
    for tool in &listed.tools {
        assert_eq!(
            Value::Object(tool.input_schema.as_ref().clone()),
            serde_json::json!({ "type": "object" }),
            "{}",
            tool.name
        );
    }
    for name in ["say", "session_status"] {
        let called = tokio::time::timeout(
            Duration::from_secs(15),
            running.call_tool(CallToolRequestParam {
                name: (*name).into(),
                arguments: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("{name} tools/call timed out"));
        match called {
            Err(ServiceError::McpError(err)) => {
                assert_eq!(err.code, ErrorCode::METHOD_NOT_FOUND, "{name}: {err}");
            }
            Ok(result) => panic!("{name} tools/call returned a body: {result:?}"),
            Err(err) => panic!("{name} tools/call: {err}"),
        }
    }
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}

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

struct Derived {
    kernel: Vec<Value>,
    rejected: Vec<(String, String)>,
    npc_places: Vec<(String, String)>,
}

fn wait_count(slot: &Arc<Mutex<String>>, needle: &str, count: usize, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = text_of(slot);
        if text.matches(needle).count() >= count {
            return text;
        }
        if start.elapsed() > timeout {
            panic!("saw {} of {needle}, wanted {count}\n{text}", text.matches(needle).count());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_has(slot: &Arc<Mutex<String>>, needle: &str, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = text_of(slot);
        if text.contains(needle) {
            return text;
        }
        if start.elapsed() > timeout {
            panic!("missing {needle} after {timeout:?}\n{text}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_exit(proc: &mut Proc, timeout: Duration) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = proc.child.try_wait().unwrap() {
            return status;
        }
        if start.elapsed() > timeout {
            panic!(
                "process still running\nstdout:\n{}\nstderr:\n{}",
                text_of(&proc.stdout),
                text_of(&proc.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_humans(pack: &Path, humans: u32, timeout: Duration) -> spookery_kernel::model::Now {
    let start = Instant::now();
    loop {
        if let Ok(shard) = spookery_kernel::open_shard(pack) {
            if shard.now().presence.humans == humans {
                return shard.now().clone();
            }
        }
        if start.elapsed() > timeout {
            panic!("humans did not become {humans}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn write_stdin(proc: &mut Proc, line: &str) {
    let stdin = proc.stdin.as_mut().unwrap();
    writeln!(stdin, "{line}").unwrap();
    stdin.flush().unwrap();
}

fn now_of(pack: &Path) -> spookery_kernel::model::Now {
    spookery_kernel::open_shard(pack)
        .unwrap_or_else(|err| panic!("open: {err}"))
        .now()
        .clone()
}

fn tool_body(stream: &mut TcpStream, id: u64, name: &str, arguments: &str) -> Value {
    let lines = call(stream, id, name, arguments);
    let line = result_line(&lines, id);
    assert!(line.contains("\"result\""), "{name}: {line}");
    structured(line)
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

fn kernel_intents(tick: &Value) -> Vec<Value> {
    let mut rows: Vec<Value> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["source"] == "kernel")
        .cloned()
        .collect();
    rows.sort_by(|left, right| left["id"].as_str().unwrap().cmp(right["id"].as_str().unwrap()));
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

fn assert_closed_effects(tick: &Value, travel_id: &str) {
    let rejected: Vec<_> = tick["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["id"] == travel_id)
        .collect();
    assert_eq!(rejected.len(), 1, "{tick}");
    assert!(
        tick["intents"].as_array().unwrap().iter().all(|intent| intent["id"] != travel_id),
        "{tick}"
    );
    assert!(
        tick["effects"].as_array().unwrap().iter().all(|effect| effect["intent_id"] != travel_id),
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

fn refuse_player(player: &mut Proc, line: &str, code: &str, seen: &mut usize) {
    let stdout = text_of(&player.stdout);
    write_stdin(player, line);
    *seen += 1;
    let stderr = wait_count(&player.stderr, code, *seen, Duration::from_secs(5));
    assert_eq!(stderr.matches(code).count(), *seen, "{line}\n{stderr}");
    assert_eq!(text_of(&player.stdout), stdout, "{line} wrote on stdout");
}

fn seat_player(pack: &Path) -> Proc {
    let player = spawn(&["play", &pack_arg(pack)]);
    wait_has(&player.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    assert!(text_of(&player.stderr).is_empty(), "{}", text_of(&player.stderr));
    player
}

#[test]
fn v02_p03_travel_queues_until_commit_or_commits_one_live_tick() {
    let _lock = lock_port();
    let baseline = {
        let pack = temp_pack("b-wait");
        let mut server = spawn(&["run", &pack_arg(&pack)]);
        wait_listen(&mut server);
        let mut player = seat_player(&pack);
        let seated = now_of(&pack);
        assert_eq!(seated.presence.humans, 1);
        assert_eq!(seated.world_time, 1080);
        assert_eq!(rowan_place(&seated), "docks_shop");
        write_stdin(&mut player, "wait");
        wait_count(
            &player.stdout,
            "\"kind\":\"resume\"",
            text_of(&player.stdout).matches("\"kind\":\"resume\"").count() + 1,
            Duration::from_secs(5),
        );
        let now = now_of(&pack);
        assert_eq!(now.world_time, 1085);
        assert_eq!(now.presence.humans, 1);
        assert_eq!(rowan_place(&now), "docks_shop");
        assert_eq!(tick_names(&pack), vec!["tick-0001.jsonl".to_string()]);
        let tick = tick_value(&pack, "tick-0001.jsonl");
        assert!(tick["intents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|intent| intent["move"] == "wait" && intent["source"] == "player"));
        let derived = derived(&tick, &now, &[]);
        let _ = fs::remove_dir_all(&pack);
        derived
    };

    let seated = {
        let pack = temp_pack("b-seated");
        let mut server = spawn(&["run", &pack_arg(&pack)]);
        wait_listen(&mut server);
        let mut gm = gm_session();
        let mut next = 2u64;
        let mut player = seat_player(&pack);
        let opened = now_of(&pack);
        assert_eq!(opened.presence.humans, 1);
        assert_eq!(opened.world_time, 1080);
        assert_eq!(rowan_place(&opened), "docks_shop");
        assert!(tick_names(&pack).is_empty());

        let mut schema = 0usize;
        let mut spec = 0usize;
        for line in ["travel", "travel mira_rooms extra", "travel  mira_rooms", " wait"] {
            refuse_player(&mut player, line, "SCHEMA", &mut schema);
        }
        for line in ["attack", "attack mira", "give", "take", "dance"] {
            refuse_player(&mut player, line, "NOT_IN_SPEC", &mut spec);
        }
        let held = now_of(&pack);
        assert_eq!(held.world_time, 1080);
        assert_eq!(held.tick_index, 0);
        assert_eq!(rowan_place(&held), "docks_shop");
        assert!(tick_names(&pack).is_empty());
        let idle = tool_body(&mut gm, next, "session_status", "{}");
        next += 1;
        assert!(queued_rows(&idle).is_empty(), "{idle}");

        let before = text_of(&player.stdout);
        write_stdin(&mut player, "travel mira_rooms");
        let stdout = wait_has(&player.stdout, "\"kind\":\"queued\"", Duration::from_secs(5));
        let added = &stdout[before.len()..];
        assert!(added.contains("\"kind\":\"queued\""), "{added}");
        assert!(!added.contains("\"kind\":\"resume\""), "{added}");
        assert!(!added.contains("\"kind\":\"refused\""), "{added}");
        let travel_id = queued_ids(added);
        assert_eq!(travel_id.len(), 1, "{added}");
        let travel_id = travel_id[0].clone();
        let queued_now = now_of(&pack);
        assert_eq!(queued_now.world_time, 1080);
        assert_eq!(queued_now.tick_index, 0);
        assert_eq!(rowan_place(&queued_now), "docks_shop");
        assert!(tick_names(&pack).is_empty());
        let status = tool_body(&mut gm, next, "session_status", "{}");
        next += 1;
        assert_eq!(
            queued_rows(&status),
            vec![(travel_id.clone(), "travel".to_string())]
        );
        assert!(!fs::read(pack.join("state/now.yaml")).unwrap().windows(travel_id.len()).any(|window| window == travel_id.as_bytes()));

        let committed = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
        next += 1;
        assert!(committed.get("code").is_none(), "{committed}");
        assert!(committed["log_path"].as_str().unwrap().ends_with("tick-0001.jsonl"), "{committed}");
        let now = now_of(&pack);
        assert_eq!(now.world_time, 1085);
        assert_ne!(now.world_time, 1140);
        assert_eq!(now.tick_index, 1);
        assert_eq!(now.presence.humans, 1);
        assert_eq!(rowan_place(&now), "docks_shop");
        assert_ne!(now.last_tick_at, opened.last_tick_at);
        assert_eq!(tick_names(&pack), vec!["tick-0001.jsonl".to_string()]);
        let tick = tick_value(&pack, "tick-0001.jsonl");
        assert_eq!(tick["presence_humans"], 1);
        assert_eq!(tick["world_time_before"], 1080);
        assert_eq!(tick["world_time_after"], 1085);
        assert_eq!(tick["rejected"].as_array().unwrap().iter().find(|item| item["id"] == travel_id).unwrap()["reason"], "NOT_OWNER");
        assert_closed_effects(&tick, &travel_id);
        let derived = derived(&tick, &now, &[&travel_id]);
        assert!(tool_body(&mut gm, next, "session_status", "{}")["queued"].as_array().unwrap().is_empty());
        next += 1;

        let before_missing = text_of(&player.stdout);
        write_stdin(&mut player, "travel not_a_place");
        let missing_out = wait_count(&player.stdout, "\"kind\":\"queued\"", 2, Duration::from_secs(5));
        let missing_added = &missing_out[before_missing.len()..];
        let missing_ids = queued_ids(missing_added);
        assert_eq!(missing_ids.len(), 1, "{missing_added}");
        let missing_id = missing_ids[0].clone();
        assert_ne!(missing_id, travel_id);
        assert_eq!(now_of(&pack).world_time, 1085);
        assert_eq!(rowan_place(&now_of(&pack)), "docks_shop");
        assert_eq!(tick_names(&pack), vec!["tick-0001.jsonl".to_string()]);
        let missing_status = tool_body(&mut gm, next, "session_status", "{}");
        next += 1;
        assert_eq!(queued_rows(&missing_status), vec![(missing_id.clone(), "travel".to_string())]);

        let second = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
        assert!(second.get("code").is_none(), "{second}");
        let after = now_of(&pack);
        assert_eq!(after.world_time, 1090);
        assert_eq!(after.tick_index, 2);
        assert_eq!(after.presence.humans, 1);
        assert_eq!(rowan_place(&after), "docks_shop");
        assert_eq!(
            tick_names(&pack),
            vec!["tick-0001.jsonl".to_string(), "tick-0002.jsonl".to_string()]
        );
        let second_tick = tick_value(&pack, "tick-0002.jsonl");
        assert_eq!(second_tick["world_time_before"], 1085);
        assert_eq!(second_tick["world_time_after"], 1090);
        assert_eq!(
            second_tick["rejected"].as_array().unwrap().iter().find(|item| item["id"] == missing_id).unwrap()["reason"],
            "UNKNOWN_KEY"
        );
        assert_closed_effects(&second_tick, &missing_id);
        assert!(second_tick["rejected"].as_array().unwrap().iter().all(|item| item["id"] != travel_id));
        assert!(second_tick["intents"].as_array().unwrap().iter().all(|intent| intent["id"] != travel_id));
        let _ = fs::remove_dir_all(&pack);
        derived
    };

    let live = {
        let pack = temp_pack("b-live");
        let mut server = spawn(&["run", &pack_arg(&pack)]);
        wait_listen(&mut server);
        let mut player = seat_player(&pack);
        let seated = now_of(&pack);
        assert_eq!(seated.world_time, 1080);
        assert_eq!(rowan_place(&seated), "docks_shop");
        let mut schema = 0usize;
        let mut spec = 0usize;
        refuse_player(&mut player, "travel", "SCHEMA", &mut schema);
        refuse_player(&mut player, "travel mira_rooms extra", "SCHEMA", &mut schema);
        refuse_player(&mut player, "attack", "NOT_IN_SPEC", &mut spec);
        let held = now_of(&pack);
        assert_eq!(held.world_time, 1080);
        assert_eq!(held.tick_index, 0);
        assert!(tick_names(&pack).is_empty());
        assert_eq!(rowan_place(&held), "docks_shop");

        let before = text_of(&player.stdout);
        write_stdin(&mut player, "travel mira_rooms");
        let stdout = wait_count(
            &player.stdout,
            "\"kind\":\"resume\"",
            before.matches("\"kind\":\"resume\"").count() + 1,
            Duration::from_secs(5),
        );
        let added = &stdout[before.len()..];
        assert!(added.contains("\"kind\":\"resume\""), "{added}");
        assert!(!added.contains("\"kind\":\"queued\""), "{added}");
        let now = now_of(&pack);
        assert_eq!(now.presence.humans, 1);
        assert_eq!(now.world_time, 1085);
        assert_ne!(now.world_time, 1140);
        assert_eq!(now.tick_index, 1);
        assert_ne!(now.last_tick_at, seated.last_tick_at);
        assert_eq!(rowan_place(&now), "docks_shop");
        assert_eq!(tick_names(&pack), vec!["tick-0001.jsonl".to_string()]);
        let tick = tick_value(&pack, "tick-0001.jsonl");
        assert_eq!(tick["presence_humans"], 1);
        assert_eq!(tick["world_time_before"], 1080);
        assert_eq!(tick["world_time_after"], 1085);
        let extra: Vec<_> = rejected_except(&tick, &[])
            .into_iter()
            .filter(|row| !baseline.rejected.iter().any(|kept| kept == row))
            .collect();
        assert_eq!(extra.len(), 1, "{tick}");
        assert_eq!(extra[0].1, "NOT_OWNER");
        let travel_id = extra[0].0.clone();
        assert_closed_effects(&tick, &travel_id);
        let derived = derived(&tick, &now, &[&travel_id]);
        let _ = fs::remove_dir_all(&pack);
        derived
    };

    assert_eq!(seated.kernel, baseline.kernel);
    assert_eq!(seated.rejected, baseline.rejected);
    assert_eq!(seated.npc_places, baseline.npc_places);
    assert_eq!(live.kernel, baseline.kernel);
    assert_eq!(live.rejected, baseline.rejected);
    assert_eq!(live.npc_places, baseline.npc_places);
}

#[test]
fn v02_p04_reconnect_returns_scrollback_and_keeps_the_queue_once() {
    let _lock = lock_port();
    let pack = temp_pack("b-again");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut gm = gm_session();
    let mut next = 2u64;
    let mut player = seat_player(&pack);
    let opened = now_of(&pack);
    assert_eq!(opened.presence.humans, 1);
    assert_eq!(opened.presence.session_ids.len(), 1);
    assert_eq!(opened.presence.actors_present, vec!["pc_rowan".to_string()]);
    assert_eq!(opened.world_time, 1080);
    assert_eq!(opened.tick_index, 0);
    let first_session = opened.presence.session_ids[0].clone();

    write_stdin(&mut player, "say the tin is out");
    wait_has(&player.stdout, "the tin is out", Duration::from_secs(5));
    assert_eq!(now_of(&pack).world_time, 1080);
    assert!(tick_names(&pack).is_empty());

    write_stdin(&mut player, "wait");
    let waited = wait_count(&player.stdout, "\"kind\":\"queued\"", 1, Duration::from_secs(5));
    write_stdin(&mut player, "travel mira_rooms");
    let queued_out = wait_count(&player.stdout, "\"kind\":\"queued\"", 2, Duration::from_secs(5));
    let ids = queued_ids(&queued_out);
    assert_eq!(ids.len(), 2, "{queued_out}");
    assert_eq!(queued_ids(&waited), vec![ids[0].clone()]);
    let wait_id = ids[0].clone();
    let travel_id = ids[1].clone();
    assert_ne!(wait_id, travel_id);
    let queued = now_of(&pack);
    assert_eq!(queued.world_time, 1080);
    assert_eq!(queued.tick_index, 0);
    assert_eq!(rowan_place(&queued), "docks_shop");
    assert!(tick_names(&pack).is_empty());
    let status = tool_body(&mut gm, next, "session_status", "{}");
    next += 1;
    assert_eq!(status["presence"]["humans"], 1);
    assert_eq!(
        queued_rows(&status),
        vec![(wait_id.clone(), "wait".to_string()), (travel_id.clone(), "travel".to_string())]
    );
    let stored = fs::read(pack.join("state/now.yaml")).unwrap();
    assert!(!stored.windows(wait_id.len()).any(|window| window == wait_id.as_bytes()));
    assert!(!stored.windows(travel_id.len()).any(|window| window == travel_id.as_bytes()));

    write_stdin(&mut player, "close");
    assert_eq!(wait_exit(&mut player, Duration::from_secs(5)).code(), Some(0));
    let dropped = wait_humans(&pack, 0, Duration::from_secs(5));
    assert_eq!(dropped.world_time, 1080);
    assert_eq!(dropped.tick_index, 0);
    assert!(dropped.presence.session_ids.is_empty());
    assert!(dropped.presence.actors_present.is_empty());
    assert!(tick_names(&pack).is_empty());
    let while_down = tool_body(&mut gm, next, "session_status", "{}");
    next += 1;
    assert_eq!(while_down["presence"]["humans"], 0);
    assert_eq!(
        queued_rows(&while_down),
        vec![(wait_id.clone(), "wait".to_string()), (travel_id.clone(), "travel".to_string())]
    );

    let again = seat_player(&pack);
    let back = text_of(&again.stdout);
    let say_lines = back
        .lines()
        .filter(|line| line.contains("\"kind\":\"say\"") && line.contains("the tin is out"))
        .count();
    assert_eq!(say_lines, 1, "{back}");
    let reseated = now_of(&pack);
    assert_eq!(reseated.presence.humans, 1);
    assert_eq!(reseated.presence.actors_present, vec!["pc_rowan".to_string()]);
    assert_eq!(reseated.presence.session_ids.len(), 1);
    assert_ne!(reseated.presence.session_ids[0], first_session);
    assert_eq!(reseated.world_time, 1080);
    assert_eq!(reseated.tick_index, 0);
    assert_eq!(rowan_place(&reseated), "docks_shop");
    assert!(tick_names(&pack).is_empty());
    let tape = fs::read_to_string(pack.join("state/log/session.jsonl")).unwrap();
    assert_eq!(tape.matches("the tin is out").count(), 1, "{tape}");
    let after_hello = tool_body(&mut gm, next, "session_status", "{}");
    next += 1;
    assert_eq!(after_hello["presence"]["humans"], 1);
    assert_eq!(
        queued_rows(&after_hello),
        vec![(wait_id.clone(), "wait".to_string()), (travel_id.clone(), "travel".to_string())]
    );

    let committed = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
    next += 1;
    assert!(committed.get("code").is_none(), "{committed}");
    assert!(committed["log_path"].as_str().unwrap().ends_with("tick-0001.jsonl"), "{committed}");
    let now = now_of(&pack);
    assert_eq!(now.world_time, 1085);
    assert_ne!(now.world_time, 1140);
    assert_eq!(now.tick_index, 1);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(rowan_place(&now), "docks_shop");
    assert_eq!(tick_names(&pack), vec!["tick-0001.jsonl".to_string()]);
    let tick = tick_value(&pack, "tick-0001.jsonl");
    assert_eq!(tick["world_time_before"], 1080);
    assert_eq!(tick["world_time_after"], 1085);
    assert_eq!(tick["presence_humans"], 1);
    let waits: Vec<_> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["id"] == wait_id)
        .collect();
    assert_eq!(waits.len(), 1, "{tick}");
    assert_eq!(waits[0]["move"], "wait");
    assert_eq!(
        tick["rejected"].as_array().unwrap().iter().filter(|item| item["id"] == travel_id).count(),
        1
    );
    assert_eq!(
        tick["rejected"].as_array().unwrap().iter().find(|item| item["id"] == travel_id).unwrap()["reason"],
        "NOT_OWNER"
    );
    assert!(tick["intents"].as_array().unwrap().iter().all(|intent| intent["id"] != travel_id));
    assert_closed_effects(&tick, &travel_id);
    let drained = tool_body(&mut gm, next, "session_status", "{}");
    next += 1;
    assert!(queued_rows(&drained).is_empty(), "{drained}");

    let again_commit = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
    assert!(again_commit.get("code").is_none(), "{again_commit}");
    assert!(again_commit["log_path"].is_null(), "{again_commit}");
    let final_now = now_of(&pack);
    assert_eq!(final_now.world_time, 1085);
    assert_eq!(final_now.tick_index, 1);
    assert_eq!(final_now.presence.humans, 1);
    assert_eq!(rowan_place(&final_now), "docks_shop");
    assert_eq!(tick_names(&pack), vec!["tick-0001.jsonl".to_string()]);
    assert!(queued_rows(&tool_body(&mut gm, next + 1, "session_status", "{}")).is_empty());
    let _ = again;
    let _ = fs::remove_dir_all(&pack);
}
