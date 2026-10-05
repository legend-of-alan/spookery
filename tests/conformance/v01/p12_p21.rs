use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::support::{
    lock_port, pack_arg, spawn, temp_pack, text_of, wait_count, wait_exit, wait_has, wait_listen,
    write_stdin,
};

const GM_TOOLS: &[&str] = &[
    "session_status",
    "perceive",
    "intent",
    "say",
    "npc_act",
    "commit_turn",
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

#[test]
fn v01_no_cargo_feature_named_llm() {
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

#[test]
fn v01_p12_gm_tools_reach_perceive_and_the_refusals_write_nothing() {
    let _lock = lock_port();
    let pack = temp_pack("tools");
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
    let listed_value: Value = serde_json::from_str(&listed).unwrap();
    let tools = listed_value["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, GM_TOOLS);
    for tool in tools {
        let description = tool["description"].as_str().unwrap();
        assert!(
            description.contains("queued until commit_turn"),
            "{description}"
        );
        assert!(description.contains("does not tick"), "{description}");
        assert!(description.contains("player:<id>"), "{description}");
        assert!(description.contains("npc:<id>"), "{description}");
        assert!(description.contains("session_open"), "{description}");
        assert!(description.contains("patch_entity"), "{description}");
        assert!(description.contains("since"), "{description}");
        assert!(description.contains("player speech"), "{description}");
        assert!(description.contains("the queue"), "{description}");
    }

    let player_view = call(&mut gm, 3, "perceive", r#"{"as":"player:pc_rowan"}"#);
    let player_line = result_line(&player_view, 3);
    assert!(
        player_line.contains(
            "\"structuredContent\":{\"kind\":\"perceive\",\"viewer\":\"player:pc_rowan\""
        ),
        "{player_line}"
    );
    let player_body = structured(player_line);
    assert_eq!(player_body["kind"], "perceive");
    assert_eq!(player_body["viewer"], "player:pc_rowan");
    assert!(player_body["place"].is_string(), "{player_body}");
    assert!(player_body.get("code").is_none(), "{player_body}");
    assert!(
        player_view
            .iter()
            .all(|line| !line.contains("\"method\":\"spook/event\"")),
        "{player_view:?}"
    );

    let npc_view = structured(result_line(
        &call(&mut gm, 4, "perceive", r#"{"as":"npc:mira"}"#),
        4,
    ));
    assert_eq!(npc_view["viewer"], "npc:mira");
    assert_eq!(npc_view["kind"], "perceive");

    let unscoped = structured(result_line(
        &call(&mut gm, 5, "perceive", r#"{"as":"gm"}"#),
        5,
    ));
    assert_eq!(unscoped["code"], "UNSCOPED_PERCEIVE");
    let missing = structured(result_line(&call(&mut gm, 6, "perceive", "{}"), 6));
    assert_eq!(missing["code"], "UNSCOPED_PERCEIVE");

    let npc = structured(result_line(
        &call(&mut gm, 7, "npc_act", r#"{"actor_id":"mira"}"#),
        7,
    ));
    assert_eq!(npc, serde_json::json!({ "code": "LLM_DISABLED" }));

    let open = structured(result_line(
        &call(
            &mut gm,
            8,
            "session_open",
            r#"{"session_id":"s-gm","as":"player:pc_rowan"}"#,
        ),
        8,
    ));
    assert_eq!(open["code"], "NOT_IN_SPEC");
    let patched = call(
        &mut gm,
        9,
        "patch_entity",
        r#"{"path":"cast/mira.md","new_text":"rewritten"}"#,
    );
    let patched_line = result_line(&patched, 9);
    assert!(patched_line.contains("\"result\""), "{patched_line}");
    assert!(!patched_line.contains("-32601"), "{patched_line}");
    assert_eq!(structured(patched_line)["code"], "NOT_IN_SPEC");
    for name in [
        "resolve_check",
        "apply_effect",
        "recall",
        "session_save",
        "not_a_tool",
    ] {
        let lines = call(&mut gm, 10, name, "{}");
        let line = result_line(&lines, 10);
        assert!(line.contains("\"result\""), "{name}: {line}");
        assert!(!line.contains("-32601"), "{name}: {line}");
        assert_eq!(structured(line)["code"], "NOT_IN_SPEC", "{name}");
    }

    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    assert!(!pack.join("state/log/session.jsonl").exists());
    assert!(
        !pack.join("cast/mira.md").exists()
            || !fs::read_to_string(pack.join("cast/mira.md"))
                .unwrap()
                .contains("rewritten")
    );
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_typed_gm_line_does_not_call_say() {
    let _lock = lock_port();
    let pack = temp_pack("typed");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    let mut typed = spawn(&["gm", &pack_arg(&pack)]);
    let status = wait_exit(&mut typed, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert_eq!(text_of(&typed.stderr).trim(), "NOT_IN_SPEC");
    assert!(text_of(&typed.stdout).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_gm_say_appends_and_session_status_returns_the_tape() {
    let _lock = lock_port();
    let pack = temp_pack("say");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    let mut gm = gm_session();
    let said = call(
        &mut gm,
        3,
        "say",
        r#"{"address":"table","text":"from the chair"}"#,
    );
    let said_at = said
        .iter()
        .position(|line| line.contains("\"id\":3"))
        .unwrap();
    let event_at = said
        .iter()
        .position(|line| line.contains("\"method\":\"spook/event\""))
        .unwrap();
    assert!(said_at < event_at, "{said:?}");
    let said_line = &said[said_at];
    assert!(
        said_line.contains("\"structuredContent\":{\"kind\":\"say\",\"seq\":1,\"from\":\"gm\""),
        "{said_line}"
    );
    let body = structured(said_line);
    assert_eq!(body["kind"], "say");
    assert_eq!(body["from"], "gm");
    assert_eq!(body["address"], "table");
    assert_eq!(body["text"], "from the chair");
    assert!(body["place"].is_null());
    assert!(body["actor"].is_null());
    assert_eq!(body["recipients"], serde_json::json!([]));
    let event = &said[event_at];
    assert!(
        event.contains("\"params\":{\"kind\":\"say\",\"seq\":1,\"from\":\"gm\""),
        "{event}"
    );
    assert!(!event.contains("\"id\":3"), "{event}");
    let tape = fs::read_to_string(pack.join("state/log/session.jsonl")).unwrap();
    assert!(tape.contains("\"from\":\"gm\""), "{tape}");
    assert!(tape.contains("\"text\":\"from the chair\""), "{tape}");
    assert!(!tape.contains("\"tick_index\":1"), "{tape}");
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    assert!(server.child.try_wait().unwrap().is_none());

    let spoiled = structured(result_line(
        &call(
            &mut gm,
            4,
            "say",
            r#"{"address":"table","text":"spoof","from":"player:pc_rowan"}"#,
        ),
        4,
    ));
    assert_eq!(spoiled["code"], "SCHEMA");
    let tape_after = fs::read_to_string(pack.join("state/log/session.jsonl")).unwrap();
    assert_eq!(tape_after.lines().count(), 1, "{tape_after}");
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);

    let quiet = structured(result_line(&call(&mut gm, 5, "session_status", "{}"), 5));
    assert!(quiet.get("events").is_none(), "{quiet}");
    assert_eq!(quiet["queued"], serde_json::json!([]));
    assert_eq!(quiet["last_seq"], 1);
    assert!(quiet["presence"].is_object(), "{quiet}");
    assert!(quiet["pause"].is_object(), "{quiet}");
    assert_eq!(quiet["world_time"], 1080);
    assert_eq!(quiet["tick_index"], 0);
    assert!(quiet.get("code").is_none(), "{quiet}");

    let pulled = structured(result_line(
        &call(&mut gm, 6, "session_status", r#"{"since":0}"#),
        6,
    ));
    assert_eq!(pulled["last_seq"], 1);
    let events = pulled["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{pulled}");
    assert_eq!(events[0]["kind"], "say");
    assert_eq!(events[0]["from"], "gm");
    assert_eq!(events[0]["text"], "from the chair");
    assert!(events[0].get("tick_index").is_none(), "{events:?}");
    assert!(events[0].get("world_time").is_none(), "{events:?}");

    let bad = structured(result_line(
        &call(&mut gm, 7, "session_status", r#"{"since":0,"extra":1}"#),
        7,
    ));
    assert_eq!(bad["code"], "SCHEMA");
    assert!(bad.get("last_seq").is_none(), "{bad}");
    assert_eq!(
        fs::read_to_string(pack.join("state/log/session.jsonl")).unwrap(),
        tape_after
    );

    let mut player = spawn(&["play", &pack_arg(&pack)]);
    let hello = wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    assert_eq!(hello.matches("\"kind\":\"perceive\"").count(), 1, "{hello}");
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();
    write_stdin(
        &mut player,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"perceive","arguments":{"as":"player:pc_rowan"}}}"#,
    );
    let refused = wait_has(&player.stderr, "NOT_IN_SPEC", Duration::from_secs(5));
    assert_eq!(refused.trim(), "NOT_IN_SPEC");
    let after_call = text_of(&player.stdout);
    assert_eq!(
        after_call.matches("\"kind\":\"perceive\"").count(),
        1,
        "{after_call}"
    );
    assert!(!after_call.contains("jsonrpc"), "{after_call}");
    write_stdin(
        &mut player,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    let listed_player = wait_count(&player.stderr, "NOT_IN_SPEC", 2, Duration::from_secs(5));
    assert_eq!(
        listed_player.matches("NOT_IN_SPEC").count(),
        2,
        "{listed_player}"
    );
    assert!(
        !text_of(&player.stdout).contains("session_status"),
        "{}",
        text_of(&player.stdout)
    );
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);

    write_stdin(&mut player, "say across the table");
    wait_has(
        &player.stdout,
        "\"text\":\"across the table\"",
        Duration::from_secs(5),
    );
    let heard = call(
        &mut gm,
        8,
        "say",
        r#"{"address":"table","text":"back at you"}"#,
    );
    let heard_line = result_line(&heard, 8);
    assert!(heard_line.contains("\"from\":\"gm\""), "{heard_line}");
    let player_out = wait_has(
        &player.stdout,
        "\"text\":\"back at you\"",
        Duration::from_secs(5),
    );
    assert!(
        player_out.contains(
            "{\"kind\":\"say\",\"from\":\"gm\",\"address\":\"table\",\"text\":\"back at you\"}"
        ),
        "{player_out}"
    );
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);

    write_stdin(&mut player, "wait");
    let queued_out = wait_has(
        &player.stdout,
        "\"kind\":\"queued\"",
        Duration::from_secs(5),
    );
    let queued_id = queued_out
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["kind"] == "queued")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let status = structured(result_line(
        &call(&mut gm, 9, "session_status", r#"{"since":0}"#),
        9,
    ));
    assert!(status["last_seq"].as_u64().unwrap() >= 2, "{status}");
    let events = status["events"].as_array().unwrap();
    assert!(
        events
            .iter()
            .any(|event| event["from"] == "player:pc_rowan" && event["text"] == "across the table"),
        "{status}"
    );
    assert_eq!(status["queued"][0]["id"], queued_id);
    assert_eq!(status["queued"][0]["actor_id"], "pc_rowan");
    assert_eq!(status["queued"][0]["move"], "wait");
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);
    assert!(pack.join("state/log").read_dir().unwrap().all(|entry| {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        !name.starts_with("tick-")
    }));
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p21_stdio_bridge_copies_perceive_and_keeps_a_refused_hello_off_stdout() {
    let _lock = lock_port();
    let pack = temp_pack("stdio");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut player = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    write_stdin(&mut player, "say for the bridge");
    wait_has(
        &player.stdout,
        "\"text\":\"for the bridge\"",
        Duration::from_secs(5),
    );
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();

    let mut bridge = spawn(&["gm", &pack_arg(&pack), "--stdio"]);
    write_stdin(
        &mut bridge,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"proof","version":"0"}}}"#,
    );
    wait_has(&bridge.stdout, "\"id\":1", Duration::from_secs(5));
    write_stdin(
        &mut bridge,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    write_stdin(
        &mut bridge,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"perceive","arguments":{"as":"player:pc_rowan"}}}"#,
    );
    let perceived = wait_has(&bridge.stdout, "\"id\":2", Duration::from_secs(5));
    let perceive_line = perceived
        .lines()
        .find(|line| line.contains("\"id\":2"))
        .unwrap();
    let view = structured(perceive_line);
    assert_eq!(view["kind"], "perceive");
    assert_eq!(view["viewer"], "player:pc_rowan");
    assert!(view["place"].is_string(), "{view}");
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);

    write_stdin(
        &mut bridge,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"session_status","arguments":{"since":0}}}"#,
    );
    let status_out = wait_has(&bridge.stdout, "\"id\":3", Duration::from_secs(5));
    let status_line = status_out
        .lines()
        .find(|line| line.contains("\"id\":3"))
        .unwrap();
    let status = structured(status_line);
    assert!(status["last_seq"].as_u64().unwrap() >= 1, "{status}");
    let events = status["events"].as_array().unwrap();
    assert!(
        events.iter().any(|event| {
            event["kind"] == "say"
                && event["from"] == "player:pc_rowan"
                && event["text"] == "for the bridge"
        }),
        "{status}"
    );
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);
    assert!(bridge.child.try_wait().unwrap().is_none(), "bridge exited");

    let mut refused = spawn(&["gm", &pack_arg(&pack), "--stdio"]);
    let status = wait_exit(&mut refused, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert!(text_of(&refused.stderr).contains("{\"kind\":\"refused\",\"code\":\"SESSION_FULL\"}"));
    assert!(
        !text_of(&refused.stdout).contains("jsonrpc"),
        "{}",
        text_of(&refused.stdout)
    );
    assert!(
        text_of(&refused.stdout).is_empty(),
        "{}",
        text_of(&refused.stdout)
    );
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);
    let _ = fs::remove_dir_all(&pack);
}
