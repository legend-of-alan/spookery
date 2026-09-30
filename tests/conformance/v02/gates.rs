use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rmcp::model::{CallToolRequestParam, ErrorCode};
use rmcp::transport::TokioChildProcess;
use rmcp::{ServiceError, ServiceExt};
use serde_json::Value;
use tokio::process::Command as TokioCommand;

use super::support::{
    lock_port, pack_arg, spawn, temp_pack, text_of, wait_count, wait_exit, wait_has, wait_listen,
    write_stdin, Proc,
};

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

#[test]
fn v02_p01_gm_tools_list_names_distinct_keys_and_keeps_the_refusals() {
    let _lock = lock_port();
    let pack = temp_pack("list");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
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
        .unwrap_or_else(|| panic!("{listed}"));
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, GM_TOOLS);
    assert!(!names.contains(&"patch_entity"));

    let mut descriptions = BTreeSet::new();
    for (name, phrase) in [
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
    ] {
        let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
        let description = tool["description"].as_str().unwrap();
        assert!(description.contains(phrase), "{name}: {description}");
        assert!(
            descriptions.insert(description),
            "duplicate description for {name}"
        );
    }
    assert_eq!(descriptions.len(), GM_TOOLS.len());

    let say = &tools.iter().find(|tool| tool["name"] == "say").unwrap()["inputSchema"];
    assert_eq!(
        property_names(say),
        BTreeSet::from(["address", "text", "place", "actor"])
    );
    assert_eq!(required_list(say), ["address", "text"]);
    assert_eq!(say["additionalProperties"], false);
    assert_eq!(
        say["properties"]["address"]["enum"],
        serde_json::json!(["table", "place", "actor"])
    );
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

    let perceive = &tools
        .iter()
        .find(|tool| tool["name"] == "perceive")
        .unwrap()["inputSchema"];
    assert_eq!(
        property_names(perceive),
        BTreeSet::from(["as", "focus", "budget_tokens"])
    );
    assert_eq!(required_list(perceive), ["as"]);
    assert_eq!(perceive["additionalProperties"], false);
    assert_eq!(perceive["properties"]["as"]["type"], "string");
    assert_eq!(
        perceive["properties"]["focus"]["enum"],
        serde_json::json!(["place", "actor"])
    );
    assert_eq!(perceive["properties"]["budget_tokens"]["type"], "integer");

    let commit = &tools
        .iter()
        .find(|tool| tool["name"] == "commit_turn")
        .unwrap()["inputSchema"];
    assert_eq!(
        property_names(commit),
        BTreeSet::from(["include_tick", "session_id"])
    );
    assert_eq!(required_list(commit), ["include_tick"]);
    assert_eq!(commit["additionalProperties"], false);
    assert_eq!(commit["properties"]["include_tick"]["type"], "boolean");
    assert_eq!(commit["properties"]["session_id"]["type"], "string");

    let status = &tools
        .iter()
        .find(|tool| tool["name"] == "session_status")
        .unwrap()["inputSchema"];
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
    assert_eq!(
        required_list(intent),
        ["actor_id", "actor_kind", "source", "move"]
    );
    assert_eq!(intent["additionalProperties"], false);
    assert_eq!(intent["properties"]["source"]["const"], "player");
    assert_eq!(
        intent["properties"]["actor_kind"]["enum"],
        serde_json::json!(["npc", "player"])
    );

    let live = ["say", "perceive", "commit_turn", "session_status", "intent"];
    for left in 0..live.len() {
        for right in (left + 1)..live.len() {
            let left_schema = &tools
                .iter()
                .find(|tool| tool["name"] == live[left])
                .unwrap()["inputSchema"];
            let right_schema = &tools
                .iter()
                .find(|tool| tool["name"] == live[right])
                .unwrap()["inputSchema"];
            assert_ne!(
                left_schema, right_schema,
                "{} and {}",
                live[left], live[right]
            );
        }
    }
    for name in [
        "session_open",
        "resolve_check",
        "apply_effect",
        "npc_act",
        "recall",
        "session_save",
    ] {
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
    assert_eq!(
        refusal(&mut gm, next(), "say", r#"{"text":"hi"}"#)["code"],
        "SCHEMA"
    );
    assert_eq!(
        refusal(&mut gm, next(), "say", r#"{"address":"table"}"#)["code"],
        "SCHEMA"
    );
    assert_eq!(
        refusal(&mut gm, next(), "say", r#"{"address":"place","text":"hi"}"#)["code"],
        "SCHEMA"
    );
    assert_eq!(
        refusal(&mut gm, next(), "say", r#"{"address":"actor","text":"hi"}"#)["code"],
        "SCHEMA"
    );
    assert_eq!(
        refusal(
            &mut gm,
            next(),
            "say",
            r#"{"address":"table","text":"hi","from":"gm"}"#
        )["code"],
        "SCHEMA"
    );
    assert_eq!(
        refusal(&mut gm, next(), "perceive", "{}")["code"],
        "UNSCOPED_PERCEIVE"
    );
    assert_eq!(
        refusal(&mut gm, next(), "perceive", r#"{"focus":"place"}"#)["code"],
        "UNSCOPED_PERCEIVE"
    );
    assert_eq!(
        refusal(&mut gm, next(), "commit_turn", "{}")["code"],
        "SCHEMA"
    );
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
        assert_eq!(
            refusal(&mut gm, next(), "intent", &args.to_string())["code"],
            "SCHEMA",
            "{missing}"
        );
    }
    let status = refusal(&mut gm, next(), "session_status", "{}");
    assert!(status.get("code").is_none(), "{status}");
    assert!(status.get("queued").is_some(), "{status}");
    assert!(status.get("last_seq").is_some(), "{status}");
    assert!(status.get("events").is_none(), "{status}");
    assert_eq!(
        refusal(
            &mut gm,
            next(),
            "session_status",
            r#"{"since":0,"extra":true}"#
        )["code"],
        "SCHEMA"
    );
    assert_eq!(
        refusal(&mut gm, next(), "npc_act", "{}")["code"],
        "LLM_DISABLED"
    );
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
            refusal(&mut gm, next(), name, "{}")["code"],
            "NOT_IN_SPEC",
            "{name}"
        );
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
    let running = ()
        .serve(TokioChildProcess::new(command).unwrap_or_else(|err| panic!("spawn spook: {err}")));
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
    rows.sort_by(|left, right| {
        left["id"]
            .as_str()
            .unwrap()
            .cmp(right["id"].as_str().unwrap())
    });
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
            text_of(&player.stdout)
                .matches("\"kind\":\"resume\"")
                .count()
                + 1,
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
        for line in [
            "travel",
            "travel mira_rooms extra",
            "travel  mira_rooms",
            " wait",
        ] {
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
        let stdout = wait_has(
            &player.stdout,
            "\"kind\":\"queued\"",
            Duration::from_secs(5),
        );
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
        assert!(!fs::read(pack.join("state/now.yaml"))
            .unwrap()
            .windows(travel_id.len())
            .any(|window| window == travel_id.as_bytes()));

        let committed = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
        next += 1;
        assert!(committed.get("code").is_none(), "{committed}");
        assert!(
            committed["log_path"]
                .as_str()
                .unwrap()
                .ends_with("tick-0001.jsonl"),
            "{committed}"
        );
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
        assert_eq!(
            tick["rejected"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == travel_id)
                .unwrap()["reason"],
            "OUT_OF_RANGE"
        );
        assert_closed_effects(&tick, &travel_id);
        let derived = derived(&tick, &now, &[&travel_id]);
        assert!(tool_body(&mut gm, next, "session_status", "{}")["queued"]
            .as_array()
            .unwrap()
            .is_empty());
        next += 1;

        let before_missing = text_of(&player.stdout);
        write_stdin(&mut player, "travel not_a_place");
        let missing_out = wait_count(
            &player.stdout,
            "\"kind\":\"queued\"",
            2,
            Duration::from_secs(5),
        );
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
        assert_eq!(
            queued_rows(&missing_status),
            vec![(missing_id.clone(), "travel".to_string())]
        );

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
            second_tick["rejected"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == missing_id)
                .unwrap()["reason"],
            "UNKNOWN_KEY"
        );
        assert_closed_effects(&second_tick, &missing_id);
        assert!(second_tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["id"] != travel_id));
        assert!(second_tick["intents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|intent| intent["id"] != travel_id));
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
        refuse_player(
            &mut player,
            "travel mira_rooms extra",
            "SCHEMA",
            &mut schema,
        );
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
        assert_eq!(extra[0].1, "OUT_OF_RANGE");
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
    let waited = wait_count(
        &player.stdout,
        "\"kind\":\"queued\"",
        1,
        Duration::from_secs(5),
    );
    write_stdin(&mut player, "travel mira_rooms");
    let queued_out = wait_count(
        &player.stdout,
        "\"kind\":\"queued\"",
        2,
        Duration::from_secs(5),
    );
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
        vec![
            (wait_id.clone(), "wait".to_string()),
            (travel_id.clone(), "travel".to_string())
        ]
    );
    let stored = fs::read(pack.join("state/now.yaml")).unwrap();
    assert!(!stored
        .windows(wait_id.len())
        .any(|window| window == wait_id.as_bytes()));
    assert!(!stored
        .windows(travel_id.len())
        .any(|window| window == travel_id.as_bytes()));

    write_stdin(&mut player, "close");
    assert_eq!(
        wait_exit(&mut player, Duration::from_secs(5)).code(),
        Some(0)
    );
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
        vec![
            (wait_id.clone(), "wait".to_string()),
            (travel_id.clone(), "travel".to_string())
        ]
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
    assert_eq!(
        reseated.presence.actors_present,
        vec!["pc_rowan".to_string()]
    );
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
        vec![
            (wait_id.clone(), "wait".to_string()),
            (travel_id.clone(), "travel".to_string())
        ]
    );

    let committed = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
    next += 1;
    assert!(committed.get("code").is_none(), "{committed}");
    assert!(
        committed["log_path"]
            .as_str()
            .unwrap()
            .ends_with("tick-0001.jsonl"),
        "{committed}"
    );
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
        tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["id"] == travel_id)
            .count(),
        1
    );
    assert_eq!(
        tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == travel_id)
            .unwrap()["reason"],
        "OUT_OF_RANGE"
    );
    assert!(tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|intent| intent["id"] != travel_id));
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

#[test]
fn v02_p05_initialize_writes_no_tape_and_the_four_shapes_keep_key_order() {
    let _lock = lock_port();
    let quiet = temp_pack("c-down");
    let quiet_before = pack_snapshot(&quiet);
    let refused = TcpStream::connect("127.0.0.1:9737").unwrap_err();
    let mut bridge = spawn(&["gm", &pack_arg(&quiet), "--stdio"]);
    let bridge_status = wait_exit(&mut bridge, Duration::from_secs(5));
    assert_eq!(bridge_status.code(), Some(1));
    assert!(
        text_of(&bridge.stdout).is_empty(),
        "{}",
        text_of(&bridge.stdout)
    );
    assert_eq!(text_of(&bridge.stderr).trim(), refused.to_string());
    assert!(!text_of(&bridge.stderr).contains("NOT_IN_SPEC"));
    assert_eq!(pack_snapshot(&quiet), quiet_before);
    let _ = fs::remove_dir_all(&quiet);

    let pack = temp_pack("c-codec");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut player = seat_player(&pack);
    let clock = now_of(&pack);
    write_stdin(&mut player, "say before the chair");
    let player_line = "{\"kind\":\"say\",\"from\":\"player:pc_rowan\",\"address\":\"table\",\"text\":\"before the chair\"}";
    let said = wait_has(&player.stdout, player_line, Duration::from_secs(5));
    assert_eq!(
        said.lines().filter(|line| *line == player_line).count(),
        1,
        "{said}"
    );
    assert_keys(player_line, &["kind", "from", "address", "text"]);
    let tape_line = format!(
        "{{\"seq\":1,\"from\":\"player:pc_rowan\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"before the chair\",\"tick_index\":{},\"world_time\":{}}}",
        clock.tick_index, clock.world_time
    );
    let tape_path = pack.join("state/log/session.jsonl");
    assert_eq!(
        fs::read_to_string(&tape_path).unwrap(),
        format!("{tape_line}\n")
    );
    assert_keys(
        &tape_line,
        &[
            "seq",
            "from",
            "address",
            "place",
            "actor",
            "recipients",
            "text",
            "tick_index",
            "world_time",
        ],
    );
    let stored = fs::read(&tape_path).unwrap();
    assert_eq!(now_of(&pack).world_time, clock.world_time);
    assert_eq!(now_of(&pack).state_rev, clock.state_rev);

    let mut gm = TcpStream::connect("127.0.0.1:9737").unwrap();
    gm.set_nodelay(true).unwrap();
    gm.write_all(b"gm\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"proof\",\"version\":\"0\"}}}\n")
        .unwrap();
    gm.flush().unwrap();
    let init = read_lf(&mut gm, Duration::from_secs(5));
    assert!(init.contains("\"id\":1"), "{init}");
    assert!(init.contains("\"result\""), "{init}");
    assert!(!init.contains("spook/event"), "{init}");
    assert!(!init.contains("before the chair"), "{init}");
    assert!(!init.contains("\"seq\""), "{init}");
    assert_eq!(fs::read(&tape_path).unwrap(), stored);
    assert_eq!(fs::read_to_string(&tape_path).unwrap().lines().count(), 1);

    gm.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .unwrap();
    gm.flush().unwrap();
    let pulled = call(&mut gm, 2, "session_status", r#"{"since":0}"#);
    let pulled_line = result_line(&pulled, 2);
    let say_object = "{\"kind\":\"say\",\"seq\":1,\"from\":\"player:pc_rowan\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"before the chair\"}";
    let status_object = structured_object(pulled_line);
    assert!(
        status_object.contains(&format!("\"events\":[{say_object}]")),
        "{status_object}"
    );
    assert_keys(
        say_object,
        &[
            "kind",
            "seq",
            "from",
            "address",
            "place",
            "actor",
            "recipients",
            "text",
        ],
    );
    let status = structured(pulled_line);
    assert_eq!(status["last_seq"], 1);
    assert_eq!(status["events"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read(&tape_path).unwrap(), stored);

    write_stdin(&mut player, "say across the lane");
    let live_player = "{\"kind\":\"say\",\"from\":\"player:pc_rowan\",\"address\":\"table\",\"text\":\"across the lane\"}";
    let live_out = wait_has(&player.stdout, live_player, Duration::from_secs(5));
    assert_eq!(
        live_out.lines().filter(|line| *line == live_player).count(),
        1,
        "{live_out}"
    );
    let live_event = read_lf(&mut gm, Duration::from_secs(5));
    let live_frame = "{\"jsonrpc\":\"2.0\",\"method\":\"spook/event\",\"params\":{\"kind\":\"say\",\"seq\":2,\"from\":\"player:pc_rowan\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"across the lane\"}}";
    assert_eq!(live_event, live_frame);
    assert!(!live_event.contains("\"id\""), "{live_event}");
    assert_keys(
        &live_event,
        &[
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
        ],
    );
    let live_tape = format!(
        "{{\"seq\":2,\"from\":\"player:pc_rowan\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"across the lane\",\"tick_index\":{},\"world_time\":{}}}\n",
        clock.tick_index, clock.world_time
    );
    let tape_now = fs::read_to_string(&tape_path).unwrap();
    assert!(tape_now.ends_with(&live_tape), "{tape_now}");
    assert_eq!(tape_now.lines().count(), 2, "{tape_now}");

    let said_tool = call(
        &mut gm,
        3,
        "say",
        r#"{"address":"table","text":"from the chair"}"#,
    );
    let said_at = said_tool
        .iter()
        .position(|line| line.contains("\"id\":3"))
        .unwrap();
    let event_at = said_tool
        .iter()
        .position(|line| line.contains("\"method\":\"spook/event\""))
        .unwrap();
    assert!(said_at < event_at, "{said_tool:?}");
    assert!(said_tool
        .iter()
        .take(event_at)
        .all(|line| !line.contains("\"method\":\"spook/event\"")));
    let tool_object = "{\"kind\":\"say\",\"seq\":3,\"from\":\"gm\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"from the chair\"}";
    let tool_raw = structured_object(&said_tool[said_at]);
    assert_eq!(tool_raw, tool_object);
    assert_keys(
        tool_raw,
        &[
            "kind",
            "seq",
            "from",
            "address",
            "place",
            "actor",
            "recipients",
            "text",
        ],
    );
    let tool_event = "{\"jsonrpc\":\"2.0\",\"method\":\"spook/event\",\"params\":{\"kind\":\"say\",\"seq\":3,\"from\":\"gm\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"from the chair\"}}";
    assert_eq!(said_tool[event_at], tool_event);
    assert_eq!(fs::read_to_string(&tape_path).unwrap().lines().count(), 3);
    let _ = fs::remove_dir_all(&pack);
}

fn load_err(pack: &Path) -> spookery_kernel::Reason {
    spookery_kernel::canon::load_canon(pack).expect_err("pack load")
}

fn reject_moves(label: &str, rewrite: impl FnOnce(&str) -> String) {
    let pack = temp_pack(label);
    let path = pack.join("rules/moves.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let next = rewrite(&text);
    assert_ne!(next, text, "{label} did not change moves.yaml");
    fs::write(&path, next).unwrap();
    assert_eq!(load_err(&pack), spookery_kernel::Reason::Schema, "{label}");
    let _ = fs::remove_dir_all(&pack);
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

fn check_pack(label: &str, grit: i64) -> PathBuf {
    let pack = action_pack(label);
    let moves = pack.join("rules/moves.yaml");
    let text = fs::read_to_string(&moves).unwrap();
    let next = text.replacen(
        "  - name: mark\n    token: actor\n    on: named\n    effects:\n",
        "  - name: mark\n    token: actor\n    on: named\n    check: grit\n    effects:\n",
        1,
    );
    assert_ne!(next, text, "mark row was not given a check");
    fs::write(&moves, next).unwrap();
    let sheet = pack.join("state/actors/pc_rowan.yaml");
    let sheet_text = fs::read_to_string(&sheet).unwrap();
    let sheet_next = sheet_text.replacen(
        "stats:\n  grit: 1\n",
        &format!("stats:\n  grit: {grit}\n"),
        1,
    );
    assert!(
        sheet_next.contains(&format!("grit: {grit}\n")),
        "{sheet_text}"
    );
    fs::write(&sheet, sheet_next).unwrap();
    let canon = spookery_kernel::canon::load_canon(&pack).unwrap();
    assert_eq!(
        canon.moves.get("mark").unwrap().check.as_deref(),
        Some("grit")
    );
    stamp_pack_rev(&pack);
    pack
}

fn action_pack(label: &str) -> PathBuf {
    let pack = temp_pack(label);
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

const MARK_ROW: &str = "  - name: mark
    token: actor
    on: named
    effects:
      - class: flag
        key: marked
        value: true
";

#[test]
fn v02_p06_pack_load_rejects_a_row_that_is_not_one_flag() {
    reject_moves("p06-location-key", |text| {
        format!(
            "{text}  - name: step\n    token: place\n    on: speaker\n    effects:\n      - class: location\n        key: marked\n"
        )
    });
    reject_moves("p06-location-value", |text| {
        format!(
            "{text}  - name: step\n    token: place\n    on: speaker\n    effects:\n      - class: location\n        value: true\n"
        )
    });
    reject_moves("p06-location-other", |text| {
        format!(
            "{text}  - name: step\n    token: place\n    on: speaker\n    effects:\n      - class: location\n        delta: 1\n"
        )
    });
    reject_moves("p06-place-flag", |text| {
        format!(
            "{text}  - name: step\n    token: place\n    on: speaker\n    effects:\n      - class: flag\n        key: marked\n        value: true\n"
        )
    });
    reject_moves("p06-place-named", |text| {
        format!(
            "{text}  - name: step\n    token: place\n    on: named\n    effects:\n      - class: location\n"
        )
    });
    reject_moves("p06-actor-location", |text| {
        format!(
            "{text}  - name: mark\n    token: actor\n    on: named\n    effects:\n      - class: location\n"
        )
    });
    reject_moves("p06-none-location", |text| {
        format!(
            "{text}  - name: wave\n    token: none\n    on: speaker\n    effects:\n      - class: location\n"
        )
    });
    reject_moves("p06-actor-speaker", |text| {
        format!(
            "{text}  - name: mark\n    token: actor\n    on: speaker\n    effects:\n      - class: flag\n        key: marked\n        value: true\n"
        )
    });
    reject_moves("p06-none-named", |text| {
        format!(
            "{text}  - name: mark\n    token: none\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n"
        )
    });
    reject_moves("p06-travel-flag", |text| {
        text.replacen(
            "  - name: travel\n    token: place\n    on: speaker\n    effects:\n      - class: location\n",
            "  - name: travel\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
            1,
        )
    });
    reject_moves("p06-attack", |text| {
        text.replacen(
            "  - name: attack\n",
            "  - name: attack\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
            1,
        )
    });
    reject_moves("p06-busy", |text| {
        format!(
            "{text}  - name: mark\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: busy\n        value: true\n"
        )
    });
    reject_moves("p06-empty", |text| {
        format!("{text}  - name: mark\n    token: actor\n    on: named\n    effects: []\n")
    });
    reject_moves("p06-missing", |text| {
        format!(
            "{text}  - name: mark\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n"
        )
    });
    reject_moves("p06-extra", |text| {
        format!(
            "{text}  - name: mark\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n        delta: 1\n"
        )
    });
    {
        let pack = temp_pack("p06-check");
        let path = pack.join("rules/moves.yaml");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(
            "  - name: mark\n    token: actor\n    on: named\n    check: grit\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
        );
        fs::write(&path, text).unwrap();
        let canon = spookery_kernel::canon::load_canon(&pack).unwrap();
        let mark = canon.moves.get("mark").unwrap();
        assert_eq!(mark.check.as_deref(), Some("grit"));
        let row = mark.row.as_ref().unwrap();
        assert_eq!(row.token, spookery_kernel::moves::MoveToken::Actor);
        assert_eq!(row.on, spookery_kernel::moves::MoveOn::Named);
        assert_eq!(
            row.effects,
            vec![spookery_kernel::moves::RowEffect::Flag {
                key: "marked".to_string(),
                value: true,
            }]
        );
        let _ = fs::remove_dir_all(&pack);
    }

    let pack = temp_pack("p06-ok");
    let path = pack.join("rules/moves.yaml");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(MARK_ROW);
    text.push_str(
        "  - name: wave
    token: none
    on: speaker
    effects:
      - class: flag
        key: marked
        value: false
",
    );
    fs::write(&path, text).unwrap();
    let canon = spookery_kernel::canon::load_canon(&pack).unwrap();
    let mark = canon.moves.get("mark").unwrap();
    assert!(mark.check.is_none());
    let mark_row = mark.row.as_ref().unwrap();
    assert_eq!(mark_row.token, spookery_kernel::moves::MoveToken::Actor);
    assert_eq!(mark_row.on, spookery_kernel::moves::MoveOn::Named);
    assert_eq!(
        mark_row.effects,
        vec![spookery_kernel::moves::RowEffect::Flag {
            key: "marked".to_string(),
            value: true,
        }]
    );
    let wave = canon.moves.get("wave").unwrap().row.as_ref().unwrap();
    assert_eq!(wave.token, spookery_kernel::moves::MoveToken::None);
    assert_eq!(wave.on, spookery_kernel::moves::MoveOn::Speaker);
    assert_eq!(
        wave.effects,
        vec![spookery_kernel::moves::RowEffect::Flag {
            key: "marked".to_string(),
            value: false,
        }]
    );
    let travel = canon.moves.get("travel").unwrap().row.as_ref().unwrap();
    assert_eq!(travel.token, spookery_kernel::moves::MoveToken::Place);
    assert_eq!(travel.on, spookery_kernel::moves::MoveOn::Speaker);
    assert_eq!(
        travel.effects,
        vec![spookery_kernel::moves::RowEffect::Location]
    );
    assert!(canon.moves.get("work").unwrap().row.is_none());
    assert!(canon.moves.get("attack").unwrap().row.is_none());
    assert_eq!(canon.moves.get("attempt").unwrap().check.as_deref(), None);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v02_p07_mark_queues_until_commit_or_commits_one_live_tick() {
    let _lock = lock_port();
    {
        let pack = action_pack("row-seated");
        let mut server = spawn(&["run", &pack_arg(&pack)]);
        wait_listen(&mut server);
        let mut gm = gm_session();
        let mut next = 2u64;
        let mut player = seat_player(&pack);
        let opened = now_of(&pack);
        assert_eq!(opened.presence.humans, 1);
        assert_eq!(opened.world_time, 1080);
        assert_eq!(rowan_place(&opened), "docks_shop");
        assert!(flags_of(&opened, "mira").is_empty());

        let mut schema = 0usize;
        let mut spec = 0usize;
        for line in ["mark", "mark mira extra", "mark  mira"] {
            refuse_player(&mut player, line, "SCHEMA", &mut schema);
        }
        for line in ["attack", "attack mira", "dance"] {
            refuse_player(&mut player, line, "NOT_IN_SPEC", &mut spec);
        }
        let held = now_of(&pack);
        assert_eq!(held.world_time, 1080);
        assert_eq!(held.tick_index, 0);
        assert!(tick_names(&pack).is_empty());
        assert!(flags_of(&held, "mira").is_empty());
        let idle = tool_body(&mut gm, next, "session_status", "{}");
        next += 1;
        assert!(queued_rows(&idle).is_empty(), "{idle}");

        let before = text_of(&player.stdout);
        let stderr = text_of(&player.stderr);
        write_stdin(&mut player, "mark mira");
        let stdout = wait_has(
            &player.stdout,
            "\"kind\":\"queued\"",
            Duration::from_secs(5),
        );
        let added = &stdout[before.len()..];
        assert!(added.contains("\"kind\":\"queued\""), "{added}");
        assert!(!added.contains("\"kind\":\"resume\""), "{added}");
        assert_eq!(text_of(&player.stderr), stderr);
        let mark_id = queued_ids(added);
        assert_eq!(mark_id.len(), 1, "{added}");
        let mark_id = mark_id[0].clone();
        let queued_now = now_of(&pack);
        assert_eq!(queued_now.world_time, 1080);
        assert_eq!(queued_now.tick_index, 0);
        assert!(flags_of(&queued_now, "mira").is_empty());
        assert!(tick_names(&pack).is_empty());
        let status = tool_body(&mut gm, next, "session_status", "{}");
        next += 1;
        assert_eq!(
            queued_rows(&status),
            vec![(mark_id.clone(), "mark".to_string())]
        );

        let committed = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
        next += 1;
        assert!(committed.get("code").is_none(), "{committed}");
        let now = now_of(&pack);
        assert_eq!(now.world_time, 1085);
        assert_ne!(now.world_time, 1140);
        assert_eq!(now.tick_index, 1);
        assert_eq!(now.presence.humans, 1);
        assert_eq!(flags_of(&now, "mira"), vec!["marked".to_string()]);
        assert!(flags_of(&now, "pc_rowan").is_empty());
        assert!(flags_of(&now, "aldric").is_empty());
        assert!(flags_of(&now, "bram").is_empty());
        assert_eq!(rowan_place(&now), "docks_shop");
        let tick = tick_value(&pack, "tick-0001.jsonl");
        assert_eq!(tick["world_time_before"], 1080);
        assert_eq!(tick["world_time_after"], 1085);
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
        let flags = flag_effects(&tick);
        assert_eq!(flags.len(), 1, "{tick}");
        assert_eq!(flags[0]["as"], "player:pc_rowan");
        assert_eq!(flags[0]["intent_id"], mark_id);
        assert_eq!(flags[0]["payload"]["actor_id"], "mira");
        assert_eq!(flags[0]["payload"]["key"], "marked");
        assert_eq!(flags[0]["payload"]["value"], true);
        assert_no_derived_row(&tick, &["mark", "wave"]);
        assert!(tool_body(&mut gm, next, "session_status", "{}")["queued"]
            .as_array()
            .unwrap()
            .is_empty());
        next += 1;

        let before_missing = text_of(&player.stdout);
        write_stdin(&mut player, "mark nobody");
        let missing_out = wait_count(
            &player.stdout,
            "\"kind\":\"queued\"",
            2,
            Duration::from_secs(5),
        );
        let missing_added = &missing_out[before_missing.len()..];
        let missing_ids = queued_ids(missing_added);
        assert_eq!(missing_ids.len(), 1, "{missing_added}");
        let missing_id = missing_ids[0].clone();
        assert_ne!(missing_id, mark_id);
        assert_eq!(now_of(&pack).world_time, 1085);
        assert!(flags_of(&now_of(&pack), "mira") == vec!["marked".to_string()]);
        let missing_status = tool_body(&mut gm, next, "session_status", "{}");
        next += 1;
        assert_eq!(
            queued_rows(&missing_status),
            vec![(missing_id.clone(), "mark".to_string())]
        );
        let second = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
        next += 1;
        assert!(second.get("code").is_none(), "{second}");
        let after_missing = now_of(&pack);
        assert_eq!(after_missing.world_time, 1090);
        assert_eq!(after_missing.tick_index, 2);
        assert!(!after_missing.actors.contains_key("nobody"));
        assert_eq!(flags_of(&after_missing, "mira"), vec!["marked".to_string()]);
        let missing_tick = tick_value(&pack, "tick-0002.jsonl");
        assert_eq!(
            missing_tick["rejected"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == missing_id)
                .unwrap()["reason"],
            "UNKNOWN_KEY"
        );
        assert!(flag_effects(&missing_tick).is_empty(), "{missing_tick}");
        assert!(missing_tick["intents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|intent| intent["id"] != missing_id));

        refuse_player(&mut player, "travel", "SCHEMA", &mut schema);
        let before_travel = text_of(&player.stdout);
        write_stdin(&mut player, "travel mira_rooms");
        let travel_out = wait_count(
            &player.stdout,
            "\"kind\":\"queued\"",
            3,
            Duration::from_secs(5),
        );
        let travel_added = &travel_out[before_travel.len()..];
        let travel_ids = queued_ids(travel_added);
        assert_eq!(travel_ids.len(), 1, "{travel_added}");
        let travel_id = travel_ids[0].clone();
        assert_eq!(now_of(&pack).world_time, 1090);
        assert_eq!(rowan_place(&now_of(&pack)), "docks_shop");
        assert_eq!(
            queued_rows(&tool_body(&mut gm, next, "session_status", "{}")),
            vec![(travel_id.clone(), "travel".to_string())]
        );
        next += 1;
        let traveled = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
        assert!(traveled.get("code").is_none(), "{traveled}");
        let after_travel = now_of(&pack);
        assert_eq!(after_travel.world_time, 1095);
        assert_eq!(after_travel.tick_index, 3);
        assert_eq!(rowan_place(&after_travel), "docks_shop");
        assert_eq!(flags_of(&after_travel, "mira"), vec!["marked".to_string()]);
        let travel_tick = tick_value(&pack, "tick-0003.jsonl");
        assert_eq!(
            travel_tick["rejected"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == travel_id)
                .unwrap()["reason"],
            "OUT_OF_RANGE"
        );
        assert_closed_effects(&travel_tick, &travel_id);
        let _ = fs::remove_dir_all(&pack);
    }

    {
        let pack = action_pack("row-live");
        let mut server = spawn(&["run", &pack_arg(&pack)]);
        wait_listen(&mut server);
        let mut player = seat_player(&pack);
        assert_eq!(now_of(&pack).world_time, 1080);
        assert!(flags_of(&now_of(&pack), "mira").is_empty());
        let mut schema = 0usize;
        let mut spec = 0usize;
        refuse_player(&mut player, "mark", "SCHEMA", &mut schema);
        refuse_player(&mut player, "mark mira extra", "SCHEMA", &mut schema);
        refuse_player(&mut player, "attack", "NOT_IN_SPEC", &mut spec);
        let held = now_of(&pack);
        assert_eq!(held.world_time, 1080);
        assert_eq!(held.tick_index, 0);
        assert!(tick_names(&pack).is_empty());
        assert!(flags_of(&held, "mira").is_empty());

        let before = text_of(&player.stdout);
        write_stdin(&mut player, "mark mira");
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
        assert_eq!(flags_of(&now, "mira"), vec!["marked".to_string()]);
        assert!(flags_of(&now, "pc_rowan").is_empty());
        assert_eq!(rowan_place(&now), "docks_shop");
        let tick = tick_value(&pack, "tick-0001.jsonl");
        assert_eq!(tick["world_time_before"], 1080);
        assert_eq!(tick["world_time_after"], 1085);
        let flags = flag_effects(&tick);
        assert_eq!(flags.len(), 1, "{tick}");
        assert_eq!(flags[0]["payload"]["actor_id"], "mira");
        assert_eq!(flags[0]["payload"]["key"], "marked");
        assert_no_derived_row(&tick, &["mark", "wave"]);
        let _ = fs::remove_dir_all(&pack);
    }
}

#[test]
fn v02_p08_an_unlisted_flag_is_unknown_and_derivation_skips_the_row() {
    let _lock = lock_port();
    let pack = action_pack("row-unknown");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut gm = gm_session();
    let mut next = 2u64;
    let mut player = seat_player(&pack);
    assert_eq!(now_of(&pack).world_time, 1080);
    assert!(flags_of(&now_of(&pack), "mira").is_empty());
    assert!(flags_of(&now_of(&pack), "aldric").is_empty());
    assert!(flags_of(&now_of(&pack), "pc_rowan").is_empty());

    let mut schema = 0usize;
    refuse_player(&mut player, "wave extra", "SCHEMA", &mut schema);
    let before_wave = text_of(&player.stdout);
    write_stdin(&mut player, "wave");
    let wave_out = wait_has(
        &player.stdout,
        "\"kind\":\"queued\"",
        Duration::from_secs(5),
    );
    let wave_added = &wave_out[before_wave.len()..];
    let wave_ids = queued_ids(wave_added);
    assert_eq!(wave_ids.len(), 1, "{wave_added}");
    let wave_id = wave_ids[0].clone();
    assert_eq!(now_of(&pack).world_time, 1080);
    assert!(tick_names(&pack).is_empty());
    assert!(flags_of(&now_of(&pack), "pc_rowan").is_empty());
    assert!(flags_of(&now_of(&pack), "mira").is_empty());
    assert_eq!(
        queued_rows(&tool_body(&mut gm, next, "session_status", "{}")),
        vec![(wave_id.clone(), "wave".to_string())]
    );
    next += 1;
    let waved = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
    next += 1;
    assert!(waved.get("code").is_none(), "{waved}");
    let after_wave = now_of(&pack);
    assert_eq!(after_wave.world_time, 1085);
    assert!(flags_of(&after_wave, "pc_rowan").is_empty());
    assert!(flags_of(&after_wave, "mira").is_empty());
    assert!(flags_of(&after_wave, "aldric").is_empty());
    let wave_tick = tick_value(&pack, "tick-0001.jsonl");
    assert_eq!(
        wave_tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == wave_id)
            .unwrap()["reason"],
        "UNKNOWN_KEY"
    );
    assert!(flag_effects(&wave_tick).is_empty(), "{wave_tick}");
    assert!(wave_tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|intent| intent["move"] != "wave" && intent["move"] != "mark"));
    assert_no_derived_row(&wave_tick, &["mark", "wave"]);

    let before_mark = text_of(&player.stdout);
    write_stdin(&mut player, "mark aldric");
    let mark_out = wait_count(
        &player.stdout,
        "\"kind\":\"queued\"",
        2,
        Duration::from_secs(5),
    );
    let mark_added = &mark_out[before_mark.len()..];
    let mark_ids = queued_ids(mark_added);
    assert_eq!(mark_ids.len(), 1, "{mark_added}");
    let mark_id = mark_ids[0].clone();
    assert_eq!(now_of(&pack).world_time, 1085);
    assert!(flags_of(&now_of(&pack), "aldric").is_empty());
    let marked = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
    assert!(marked.get("code").is_none(), "{marked}");
    let after_mark = now_of(&pack);
    assert_eq!(after_mark.world_time, 1090);
    assert_eq!(after_mark.tick_index, 2);
    assert!(flags_of(&after_mark, "aldric").is_empty());
    assert!(flags_of(&after_mark, "mira").is_empty());
    assert!(flags_of(&after_mark, "pc_rowan").is_empty());
    assert!(flags_of(&after_mark, "bram").is_empty());
    let mark_tick = tick_value(&pack, "tick-0002.jsonl");
    assert_eq!(
        mark_tick["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == mark_id)
            .unwrap()["reason"],
        "UNKNOWN_KEY"
    );
    assert!(flag_effects(&mark_tick).is_empty(), "{mark_tick}");
    assert!(mark_tick["effects"]
        .as_array()
        .unwrap()
        .iter()
        .all(|effect| effect["intent_id"] != mark_id));
    assert!(mark_tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|intent| intent["move"] != "mark" && intent["id"] != mark_id));
    assert_no_derived_row(&mark_tick, &["mark", "wave"]);
    let _ = fs::remove_dir_all(&pack);
}

fn last_perceive(stdout: &str) -> Value {
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.contains("\"kind\":\"perceive\""))
        .unwrap_or_else(|| panic!("no perceive line in {stdout}"));
    serde_json::from_str(line).unwrap_or_else(|err| panic!("perceive: {err} in {line}"))
}

fn assert_street(stdout: &str, now: &spookery_kernel::model::Now) {
    let view = last_perceive(stdout);
    assert_eq!(view["kind"], "perceive");
    assert_eq!(view["place"], "docks_street");
    assert_eq!(rowan_place(now), "docks_street");
    let mut visible: Vec<String> = view["visible_actors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    visible.sort();
    let mut there: Vec<String> = now
        .actors
        .iter()
        .filter(|(_, actor)| actor.place() == "docks_street")
        .map(|(id, _)| id.clone())
        .collect();
    there.sort();
    assert!(visible.iter().any(|id| id == "pc_rowan"), "{view}");
    assert_eq!(visible, there, "{view}");
}

fn pc_location_count(tick: &Value) -> usize {
    tick["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| {
            effect["class"] == "location" && effect["payload"]["actor_id"] == "pc_rowan"
        })
        .count()
}

fn assert_street_effect(tick: &Value, intent_id: &str) {
    let hits: Vec<_> = tick["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| {
            effect["class"] == "location" && effect["payload"]["actor_id"] == "pc_rowan"
        })
        .collect();
    assert_eq!(hits.len(), 1, "{tick}");
    assert_eq!(hits[0]["intent_id"], intent_id);
    assert_eq!(hits[0]["as"], "player:pc_rowan");
    assert_eq!(hits[0]["payload"]["from"], "docks_shop");
    assert_eq!(hits[0]["payload"]["to"], "docks_street");
}

fn assert_no_pc_kernel(tick: &Value) {
    let tick_index = tick["tick_index"].as_u64().unwrap();
    for move_name in ["travel", "work", "rest", "restock", "wait"] {
        let id = spookery_kernel::derive::kernel_intent_id(
            "docks-001",
            tick_index,
            "pc_rowan",
            move_name,
        );
        assert!(
            tick["intents"]
                .as_array()
                .unwrap()
                .iter()
                .all(|intent| intent["id"] != id),
            "derived {move_name}"
        );
        assert!(
            tick["rejected"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["id"] != id),
            "rejected derived {move_name}"
        );
    }
    assert!(tick["intents"].as_array().unwrap().iter().all(|intent| {
        intent["actor_id"] != "pc_rowan" || intent["source"] != "kernel"
    }));
}

fn player_rejections(tick: &Value) -> Vec<&Value> {
    tick["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["id"].as_str().is_some_and(|id| id.contains('-')))
        .collect()
}

#[test]
fn v02_p09_rowan_travels_to_the_street() {
    let _lock = lock_port();
    {
        let pack = temp_pack("street-gm");
        let mut server = spawn(&["run", &pack_arg(&pack)]);
        wait_listen(&mut server);
        let mut gm = gm_session();
        let mut next = 2u64;
        let mut player = seat_player(&pack);
        let opened = now_of(&pack);
        assert_eq!(opened.presence.humans, 1);
        assert_eq!(opened.world_time, 1080);
        assert_eq!(opened.tick_index, 0);
        assert_eq!(rowan_place(&opened), "docks_shop");
        assert!(tick_names(&pack).is_empty());
        assert_eq!(last_perceive(&text_of(&player.stdout))["place"], "docks_shop");

        write_stdin(&mut player, "say the tin is out");
        wait_has(&player.stdout, "the tin is out", Duration::from_secs(5));
        write_stdin(&mut player, "talk mira good evening");
        wait_has(&player.stdout, "good evening", Duration::from_secs(5));
        let spoken = now_of(&pack);
        assert_eq!(spoken.world_time, 1080);
        assert_eq!(spoken.tick_index, 0);
        assert_eq!(rowan_place(&spoken), "docks_shop");
        assert!(tick_names(&pack).is_empty());

        let before_wait = text_of(&player.stdout);
        write_stdin(&mut player, "wait");
        let waited = wait_has(
            &player.stdout,
            "\"kind\":\"queued\"",
            Duration::from_secs(5),
        );
        let wait_added = &waited[before_wait.len()..];
        assert!(!wait_added.contains("\"kind\":\"resume\""), "{wait_added}");
        let wait_ids = queued_ids(wait_added);
        assert_eq!(wait_ids.len(), 1, "{wait_added}");
        let wait_id = wait_ids[0].clone();
        assert_eq!(now_of(&pack).world_time, 1080);
        assert_eq!(rowan_place(&now_of(&pack)), "docks_shop");
        assert!(tick_names(&pack).is_empty());
        assert_eq!(
            queued_rows(&tool_body(&mut gm, next, "session_status", "{}")),
            vec![(wait_id.clone(), "wait".to_string())]
        );
        next += 1;
        let waited_commit = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
        next += 1;
        assert!(waited_commit.get("code").is_none(), "{waited_commit}");
        let after_wait = now_of(&pack);
        assert_eq!(after_wait.world_time, 1085);
        assert_eq!(after_wait.tick_index, 1);
        assert_eq!(rowan_place(&after_wait), "docks_shop");
        let wait_tick = tick_value(&pack, "tick-0001.jsonl");
        assert!(wait_tick["intents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|intent| {
                intent["id"] == wait_id && intent["move"] == "wait" && intent["source"] == "player"
            }));
        assert_no_pc_kernel(&wait_tick);

        let before_mira = text_of(&player.stdout);
        write_stdin(&mut player, "travel mira_rooms");
        let mira_out = wait_count(
            &player.stdout,
            "\"kind\":\"queued\"",
            2,
            Duration::from_secs(5),
        );
        let mira_added = &mira_out[before_mira.len()..];
        assert!(!mira_added.contains("\"kind\":\"resume\""), "{mira_added}");
        let mira_ids = queued_ids(mira_added);
        assert_eq!(mira_ids.len(), 1, "{mira_added}");
        let mira_id = mira_ids[0].clone();
        assert_eq!(now_of(&pack).world_time, 1085);
        assert_eq!(rowan_place(&now_of(&pack)), "docks_shop");
        assert_eq!(tick_names(&pack), vec!["tick-0001.jsonl".to_string()]);
        let refused = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
        next += 1;
        assert!(refused.get("code").is_none(), "{refused}");
        let after_mira = now_of(&pack);
        assert_eq!(after_mira.world_time, 1090);
        assert_eq!(after_mira.tick_index, 2);
        assert_eq!(rowan_place(&after_mira), "docks_shop");
        let mira_tick = tick_value(&pack, "tick-0002.jsonl");
        assert_eq!(
            mira_tick["rejected"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == mira_id)
                .unwrap()["reason"],
            "OUT_OF_RANGE"
        );
        assert_eq!(pc_location_count(&mira_tick), 0);
        assert_no_pc_kernel(&mira_tick);

        let before_street = text_of(&player.stdout);
        write_stdin(&mut player, "travel docks_street");
        let street_out = wait_count(
            &player.stdout,
            "\"kind\":\"queued\"",
            3,
            Duration::from_secs(5),
        );
        let street_added = &street_out[before_street.len()..];
        assert!(!street_added.contains("\"kind\":\"resume\""), "{street_added}");
        assert!(
            !street_added.contains("\"place\":\"docks_street\""),
            "{street_added}"
        );
        let street_ids = queued_ids(street_added);
        assert_eq!(street_ids.len(), 1, "{street_added}");
        let street_id = street_ids[0].clone();
        let queued_now = now_of(&pack);
        assert_eq!(queued_now.world_time, 1090);
        assert_eq!(queued_now.tick_index, 2);
        assert_eq!(rowan_place(&queued_now), "docks_shop");
        assert_eq!(
            queued_rows(&tool_body(&mut gm, next, "session_status", "{}")),
            vec![(street_id.clone(), "travel".to_string())]
        );
        next += 1;
        let before_see = text_of(&player.stdout);
        let moved = tool_body(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
        assert!(moved.get("code").is_none(), "{moved}");
        let seen = wait_has(
            &player.stdout,
            "\"place\":\"docks_street\"",
            Duration::from_secs(5),
        );
        let seen_added = &seen[before_see.len()..];
        assert!(seen_added.contains("\"kind\":\"perceive\""), "{seen_added}");
        let arrived = now_of(&pack);
        assert_eq!(arrived.world_time, 1095);
        assert_eq!(arrived.world_time, queued_now.world_time + 5);
        assert_eq!(arrived.tick_index, 3);
        assert_eq!(arrived.presence.humans, 1);
        assert_street(&seen, &arrived);
        let street_tick = tick_value(&pack, "tick-0003.jsonl");
        assert_eq!(street_tick["world_time_before"], 1090);
        assert_eq!(street_tick["world_time_after"], 1095);
        assert_street_effect(&street_tick, &street_id);
        assert_no_pc_kernel(&street_tick);
        let _ = fs::remove_dir_all(&pack);
    }

    {
        let pack = temp_pack("street-live");
        let mut server = spawn(&["run", &pack_arg(&pack)]);
        wait_listen(&mut server);
        let mut player = seat_player(&pack);
        assert_eq!(now_of(&pack).world_time, 1080);
        assert_eq!(rowan_place(&now_of(&pack)), "docks_shop");
        assert_eq!(last_perceive(&text_of(&player.stdout))["place"], "docks_shop");

        write_stdin(&mut player, "say the tin is out");
        wait_has(&player.stdout, "the tin is out", Duration::from_secs(5));
        write_stdin(&mut player, "talk mira good evening");
        wait_has(&player.stdout, "good evening", Duration::from_secs(5));
        assert_eq!(now_of(&pack).world_time, 1080);
        assert_eq!(now_of(&pack).tick_index, 0);
        assert_eq!(rowan_place(&now_of(&pack)), "docks_shop");
        assert!(tick_names(&pack).is_empty());

        let before_wait = text_of(&player.stdout);
        write_stdin(&mut player, "wait");
        let waited = wait_count(
            &player.stdout,
            "\"kind\":\"resume\"",
            before_wait.matches("\"kind\":\"resume\"").count() + 1,
            Duration::from_secs(5),
        );
        let wait_added = &waited[before_wait.len()..];
        assert!(wait_added.contains("\"kind\":\"resume\""), "{wait_added}");
        assert!(!wait_added.contains("\"kind\":\"queued\""), "{wait_added}");
        let after_wait = now_of(&pack);
        assert_eq!(after_wait.world_time, 1085);
        assert_eq!(after_wait.tick_index, 1);
        assert_eq!(rowan_place(&after_wait), "docks_shop");

        let before_mira = text_of(&player.stdout);
        write_stdin(&mut player, "travel mira_rooms");
        let mira_out = wait_count(
            &player.stdout,
            "\"kind\":\"resume\"",
            before_mira.matches("\"kind\":\"resume\"").count() + 1,
            Duration::from_secs(5),
        );
        let mira_added = &mira_out[before_mira.len()..];
        assert!(!mira_added.contains("\"kind\":\"queued\""), "{mira_added}");
        let after_mira = now_of(&pack);
        assert_eq!(after_mira.world_time, 1090);
        assert_eq!(after_mira.tick_index, 2);
        assert_eq!(rowan_place(&after_mira), "docks_shop");
        let mira_tick = tick_value(&pack, "tick-0002.jsonl");
        assert_eq!(mira_tick["world_time_before"], 1085);
        assert_eq!(mira_tick["world_time_after"], 1090);
        let refused = player_rejections(&mira_tick);
        assert_eq!(refused.len(), 1, "{mira_tick}");
        assert_eq!(refused[0]["reason"], "OUT_OF_RANGE");
        assert_eq!(pc_location_count(&mira_tick), 0);
        assert!(mira_tick["intents"].as_array().unwrap().iter().all(|intent| {
            intent["actor_id"] != "pc_rowan" || intent["move"] != "travel"
        }));
        assert_no_pc_kernel(&mira_tick);

        let before_street = text_of(&player.stdout);
        write_stdin(&mut player, "travel docks_street");
        let seen = wait_has(
            &player.stdout,
            "\"place\":\"docks_street\"",
            Duration::from_secs(5),
        );
        let street_added = &seen[before_street.len()..];
        assert!(street_added.contains("\"kind\":\"resume\""), "{street_added}");
        assert!(street_added.contains("\"kind\":\"perceive\""), "{street_added}");
        assert!(!street_added.contains("\"kind\":\"queued\""), "{street_added}");
        let arrived = now_of(&pack);
        assert_eq!(arrived.presence.humans, 1);
        assert_eq!(arrived.world_time, 1095);
        assert_eq!(arrived.world_time, after_mira.world_time + 5);
        assert_eq!(arrived.tick_index, 3);
        assert_street(&seen, &arrived);
        let street_tick = tick_value(&pack, "tick-0003.jsonl");
        assert_eq!(street_tick["world_time_before"], 1090);
        assert_eq!(street_tick["world_time_after"], 1095);
        let travel = street_tick["intents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|intent| {
                intent["actor_id"] == "pc_rowan"
                    && intent["move"] == "travel"
                    && intent["source"] == "player"
            })
            .unwrap_or_else(|| panic!("no travel intent in {street_tick}"));
        assert_eq!(travel["target"]["place"], "docks_street");
        assert_street_effect(&street_tick, travel["id"].as_str().unwrap());
        assert_no_pc_kernel(&street_tick);
        let _ = fs::remove_dir_all(&pack);
    }
}

#[test]
fn v02_p10_a_row_names_a_check_and_the_templates_run_either_way() {
    let _lock = lock_port();
    play_checked_mark("check-pass", 1, true);
    play_checked_mark("check-fail", -20, false);
}

fn play_checked_mark(label: &str, grit: i64, success: bool) {
    let pack = check_pack(label, grit);
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut player = seat_player(&pack);
    let opened = now_of(&pack);
    assert_eq!(opened.presence.humans, 1);
    assert_eq!(opened.world_time, 1080);
    assert_eq!(opened.tick_index, 0);
    assert_eq!(rowan_place(&opened), "docks_shop");
    assert!(flags_of(&opened, "mira").is_empty());
    assert!(tick_names(&pack).is_empty());

    let word = if success { "true" } else { "false" };
    let line = format!("{{\"kind\":\"check\",\"success\":{word}}}");
    let before = text_of(&player.stdout);
    write_stdin(&mut player, "mark mira");
    let stdout = wait_has(&player.stdout, &line, Duration::from_secs(5));
    let added = &stdout[before.len()..];
    assert!(added.contains(&line), "{added}");
    assert!(!added.contains("\"faces\""), "{added}");
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
    assert!(
        text_of(&player.stderr).is_empty(),
        "{}",
        text_of(&player.stderr)
    );
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.world_time, 1085);
    assert_eq!(now.tick_index, 1);
    assert_eq!(rowan_place(&now), "docks_shop");
    assert_eq!(flags_of(&now, "mira"), vec!["marked".to_string()]);
    assert!(flags_of(&now, "pc_rowan").is_empty());
    let tick = tick_value(&pack, "tick-0001.jsonl");
    assert_eq!(tick["world_time_before"], 1080);
    assert_eq!(tick["world_time_after"], 1085);
    assert!(tick["tool_trace"]
        .as_array()
        .unwrap()
        .iter()
        .any(|name| name == "resolve_check"));
    let marks: Vec<_> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "mark")
        .collect();
    assert_eq!(marks.len(), 1, "{tick}");
    assert_eq!(marks[0]["source"], "player");
    assert_eq!(marks[0]["actor_id"], "pc_rowan");
    assert_eq!(marks[0]["target"]["actor"], "mira");
    let flags = flag_effects(&tick);
    assert_eq!(flags.len(), 1, "{tick}");
    assert_eq!(flags[0]["intent_id"], marks[0]["id"]);
    assert_eq!(flags[0]["as"], "player:pc_rowan");
    assert_eq!(flags[0]["payload"]["actor_id"], "mira");
    assert_eq!(flags[0]["payload"]["key"], "marked");
    assert_eq!(flags[0]["payload"]["value"], true);
    assert_no_derived_row(&tick, &["mark", "wave"]);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v02_p11_a_success_list_and_a_failure_list() {
    assert_attack_resource_row();
    let _lock = lock_port();
    play_outcome_mark("lists-pass", 1, true);
    play_outcome_mark("lists-fail", -20, false);
}

fn assert_attack_resource_row() {
    let pack = temp_pack("p11-attack");
    let path = pack.join("rules/moves.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replacen("  - name: attack\n", ATTACK_ROW, 1);
    assert_ne!(next, text, "attack row was not written");
    fs::write(&path, next).unwrap();
    let canon = spookery_kernel::canon::load_canon(&pack).unwrap();
    let attack = canon.moves.get("attack").unwrap();
    assert_eq!(attack.check.as_deref(), Some("grit"));
    let row = attack.row.as_ref().unwrap();
    assert_eq!(row.token, spookery_kernel::moves::MoveToken::Actor);
    assert_eq!(row.on, spookery_kernel::moves::MoveOn::Named);
    assert!(row.effects.is_empty());
    assert_eq!(
        row.success,
        vec![spookery_kernel::moves::RowEffect::Resource {
            key: "hp".to_string(),
            delta: -1,
        }]
    );
    assert_eq!(
        row.failure,
        vec![spookery_kernel::moves::RowEffect::Resource {
            key: "hp".to_string(),
            delta: 0,
        }]
    );
    let _ = fs::remove_dir_all(&pack);
}

fn play_outcome_mark(label: &str, grit: i64, success: bool) {
    let pack = outcome_pack(label, grit);
    let canon = spookery_kernel::canon::load_canon(&pack).unwrap();
    let mark = canon.moves.get("mark").unwrap();
    assert_eq!(mark.check.as_deref(), Some("grit"));
    let row = mark.row.as_ref().unwrap();
    assert!(row.effects.is_empty());
    assert_eq!(
        row.success,
        vec![spookery_kernel::moves::RowEffect::Flag {
            key: "marked".to_string(),
            value: true,
        }]
    );
    assert_eq!(
        row.failure,
        vec![spookery_kernel::moves::RowEffect::Flag {
            key: "missed".to_string(),
            value: true,
        }]
    );
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut player = seat_player(&pack);
    let opened = now_of(&pack);
    assert_eq!(opened.presence.humans, 1);
    assert_eq!(opened.world_time, 1080);
    assert_eq!(opened.tick_index, 0);
    assert_eq!(rowan_place(&opened), "docks_shop");
    assert!(flags_of(&opened, "mira").is_empty());
    assert!(tick_names(&pack).is_empty());

    let word = if success { "true" } else { "false" };
    let line = format!("{{\"kind\":\"check\",\"success\":{word}}}");
    let before = text_of(&player.stdout);
    write_stdin(&mut player, "mark mira");
    let stdout = wait_has(&player.stdout, &line, Duration::from_secs(5));
    let added = &stdout[before.len()..];
    assert!(added.contains(&line), "{added}");
    assert!(!added.contains("\"faces\""), "{added}");
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
    assert!(
        text_of(&player.stderr).is_empty(),
        "{}",
        text_of(&player.stderr)
    );
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.world_time, 1085);
    assert_eq!(now.tick_index, 1);
    assert_eq!(rowan_place(&now), "docks_shop");
    let written = if success { "marked" } else { "missed" };
    assert_eq!(flags_of(&now, "mira"), vec![written.to_string()]);
    assert!(flags_of(&now, "pc_rowan").is_empty());
    let tick = tick_value(&pack, "tick-0001.jsonl");
    assert_eq!(tick["world_time_before"], 1080);
    assert_eq!(tick["world_time_after"], 1085);
    assert!(tick["tool_trace"]
        .as_array()
        .unwrap()
        .iter()
        .any(|name| name == "resolve_check"));
    let marks: Vec<_> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "mark")
        .collect();
    assert_eq!(marks.len(), 1, "{tick}");
    assert_eq!(marks[0]["source"], "player");
    assert_eq!(marks[0]["actor_id"], "pc_rowan");
    assert_eq!(marks[0]["target"]["actor"], "mira");
    let flags = flag_effects(&tick);
    assert_eq!(flags.len(), 1, "{tick}");
    assert_eq!(flags[0]["intent_id"], marks[0]["id"]);
    assert_eq!(flags[0]["as"], "player:pc_rowan");
    assert_eq!(flags[0]["payload"]["actor_id"], "mira");
    assert_eq!(flags[0]["payload"]["key"], written);
    assert_eq!(flags[0]["payload"]["value"], true);
    assert_no_derived_row(&tick, &["mark", "wave"]);
    let _ = fs::remove_dir_all(&pack);
}

fn outcome_pack(label: &str, grit: i64) -> PathBuf {
    let pack = temp_pack(label);
    let moves = pack.join("rules/moves.yaml");
    let mut text = fs::read_to_string(&moves).unwrap();
    text.push_str(OUTCOME_ROW);
    fs::write(&moves, text).unwrap();
    let mira = pack.join("cast/mira.md");
    let mira_text = fs::read_to_string(&mira).unwrap();
    let patched = mira_text.replacen(
        "tick:\n  eligible: true\n",
        "tick:\n  eligible: true\n  flags:\n    - marked\n    - missed\n",
        1,
    );
    assert_ne!(patched, mira_text, "mira tick block was not patched");
    fs::write(&mira, patched).unwrap();
    let sheet = pack.join("state/actors/pc_rowan.yaml");
    let sheet_text = fs::read_to_string(&sheet).unwrap();
    let sheet_next = sheet_text.replacen("stats:\n  grit: 1\n", &format!("stats:\n  grit: {grit}\n"), 1);
    assert!(
        sheet_next.contains(&format!("grit: {grit}\n")),
        "{sheet_text}"
    );
    fs::write(&sheet, sheet_next).unwrap();
    stamp_pack_rev(&pack);
    pack
}

const OUTCOME_ROW: &str = "  - name: mark
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
";

const ATTACK_ROW: &str = "  - name: attack
    token: actor
    on: named
    check: grit
    success:
      - class: resource
        key: hp
        delta: -1
    failure:
      - class: resource
        key: hp
        delta: 0
";

#[test]
fn v02_p12_a_queued_action_supplies_the_opposing_check() {
    let _lock = lock_port();
    play_opposed("oppose-open", false);
    play_opposed("oppose-held", true);
}

fn play_opposed(label: &str, opposed: bool) {
    let pack = oppose_pack(label);
    let opened = now_of(&pack);
    assert_eq!(opened.state_rev, 0);
    assert_eq!(opened.world_time, 1080);
    assert_eq!(opened.tick_index, 0);
    assert_eq!(rowan_place(&opened), "docks_shop");
    assert_eq!(
        opened.actors.get("pc_nell").unwrap().place(),
        "docks_shop"
    );
    assert!(flags_of(&opened, "pc_nell").is_empty());
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut gm = gm_session();
    let mut next = 2u64;
    let mut player = seat_player(&pack);
    assert_eq!(now_of(&pack).world_time, 1080);
    assert!(tick_names(&pack).is_empty());

    if opposed {
        let defend = tool_body(
            &mut gm,
            next,
            "intent",
            r#"{"actor_id":"pc_nell","actor_kind":"player","source":"player","move":"defend"}"#,
        );
        next += 1;
        assert!(defend.get("code").is_none(), "{defend}");
        assert!(defend
            .get("queued")
            .and_then(|id| id.as_str())
            .is_some_and(|id| !id.is_empty()));
        assert_eq!(now_of(&pack).world_time, 1080);
        assert!(tick_names(&pack).is_empty());
    }

    let before = text_of(&player.stdout);
    write_stdin(&mut player, "mark pc_nell");
    let queued_out = wait_has(&player.stdout, "\"kind\":\"queued\"", Duration::from_secs(5));
    let queued_added = &queued_out[before.len()..];
    assert!(queued_added.contains("\"kind\":\"queued\""), "{queued_added}");
    assert!(!queued_added.contains("\"kind\":\"check\""), "{queued_added}");
    assert_eq!(now_of(&pack).world_time, 1080);
    assert!(tick_names(&pack).is_empty());

    let status = tool_body(&mut gm, next, "session_status", "{}");
    next += 1;
    let queued = status["queued"].as_array().unwrap();
    if opposed {
        assert!(
            queued.iter().any(|item| {
                item["actor_id"] == "pc_nell" && item["move"] == "defend"
            }),
            "{status}"
        );
    } else {
        assert!(
            queued
                .iter()
                .all(|item| item["move"] != "defend"),
            "{status}"
        );
    }
    assert!(
        queued
            .iter()
            .any(|item| item["actor_id"] == "pc_rowan" && item["move"] == "mark"),
        "{status}"
    );
    assert_eq!(status["world_time"], 1080);

    let word = if opposed { "false" } else { "true" };
    let line = format!("{{\"kind\":\"check\",\"success\":{word}}}");
    let before_commit = text_of(&player.stdout);
    let committed_lines = call(&mut gm, next, "commit_turn", r#"{"include_tick":true}"#);
    let committed = structured(result_line(&committed_lines, next));
    assert!(committed.get("code").is_none(), "{committed}");
    let stdout = wait_has(&player.stdout, &line, Duration::from_secs(5));
    let added = &stdout[before_commit.len()..];
    assert!(added.contains(&line), "{added}");
    assert!(!added.contains("\"faces\""), "{added}");
    assert!(
        text_of(&player.stderr).is_empty(),
        "{}",
        text_of(&player.stderr)
    );

    let checks = gm_checks(&committed_lines);
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
        assert_eq!(
            flags,
            vec!["guarded".to_string(), "missed".to_string()]
        );
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
    let _ = fs::remove_dir_all(&pack);
}

fn oppose_pack(label: &str) -> PathBuf {
    let pack = temp_pack(label);
    let moves = pack.join("rules/moves.yaml");
    let mut text = fs::read_to_string(&moves).unwrap();
    text.push_str(OPPOSE_ROWS);
    fs::write(&moves, text).unwrap();
    fs::write(pack.join("cast/pc-nell.md"), NELL_CAST).unwrap();
    fs::write(pack.join("state/actors/pc_nell.yaml"), NELL_SHEET).unwrap();
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
    fs::write(
        &now_path,
        spookery_kernel::codec::emit_now(&now).unwrap(),
    )
    .unwrap();
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
