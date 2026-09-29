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
fn gm_tools_list_names_distinct_keys_and_keeps_the_refusals() {
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
async fn pack_table_stdio_does_not_gain_say_or_a_tools_call_body() {
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
