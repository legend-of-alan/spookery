use spookery_engine::testing::world_of;
use std::io::{Read, Write};
use std::net::TcpStream;

use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use rmcp::model::ProtocolVersion;
use serde_json::{json, Map, Value};
use spookery::protocol::{ServerBody, ServerMessage};

use crate::mcp_client::{GmClient, RawSession};
use crate::support;
use crate::World;

use super::host;

#[when(expr = "a coding agent calls the GM tools over loopback HTTP")]
async fn agent_calls_over_http(world: &mut World) {
    let (_tcp, http) = listen_on_free_ports(world).await;
    let mut gm = GmClient::connect(to_addr(http))
        .await
        .unwrap_or_else(|err| panic!("HTTP: {err}"));
    call_each_gm_tool(&mut gm).await;
    gm.close().await;
}

#[when(expr = "the operator runs with an open MCP HTTP address")]
async fn the_operator_runs_with_an_open_mcp_http_address(world: &mut World) {
    let pack = support::pack_arg(host::pack(world));
    host::run(world, &["run", &pack, "--mcp-addr", "0.0.0.0:9"]).await;
}

#[when(expr = "an HTTP request arrives with a public Origin")]
async fn an_http_request_arrives_with_a_public_origin(world: &mut World) {
    let (_tcp, http) = listen_on_free_ports(world).await;
    world.scratch = post_initialize(http, "127.0.0.1", Some("https://evil.example"));
}

#[then(expr = "the response is forbidden")]
fn the_response_is_forbidden(world: &mut World) {
    let status = world
        .scratch
        .lines()
        .next()
        .unwrap_or_else(|| panic!("no HTTP status\n{}", world.scratch));
    assert!(
        status.contains(" 403 "),
        "status: {status}\n{}",
        world.scratch
    );
}

#[then(expr = "a GM hello still holds the seat")]
async fn a_gm_hello_still_sits(world: &mut World) {
    host::seat_gm(world).await;
}

#[when(regex = r"^an HTTP request arrives with Host (\S+)$")]
async fn an_http_request_arrives_with_host(world: &mut World, host: String) {
    let (_tcp, http) = listen_on_free_ports(world).await;
    world.scratch = post_initialize(http, &host, None);
}

#[when(expr = "a human GM connects on the game port")]
async fn a_human_gm_connects_on_the_game_port(world: &mut World) {
    let pack = host::pack(world).to_path_buf();
    host::listen(world, &pack).await;
    let addr = host::game_addr(world);
    world.gm = Some(support::spawn(&["gm", "--addr", &addr]));
    world.mark = usize::from(host::live_mut(world).mcp_port);
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    super::common::wait_proc_until(
        gm,
        |stdout| stdout.contains("Commands:"),
        "the gm to connect",
    )
    .await;
}

#[when(expr = "a coding agent tries to connect at that MCP endpoint")]
fn a_coding_agent_tries_to_connect_at_that_mcp_endpoint(world: &mut World) {
    let mut session = RawSession::open(port(world));
    world.scratch = session.open_event_stream();
}

#[then(expr = "the agent is told the table is full")]
fn the_agent_is_told_the_table_is_full(world: &mut World) {
    assert!(
        world.scratch.contains("TABLE_FULL"),
        "refusal:\n{}",
        world.scratch
    );
}

#[when(expr = "the human GM leaves")]
fn the_human_gm_leaves(world: &mut World) {
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    let mut stdin = gm.stdin.take().unwrap_or_else(|| panic!("no gm stdin"));
    writeln!(stdin, "release").unwrap_or_else(|err| panic!("release: {err}"));
    drop(stdin);
    let gm = world.gm.as_mut().unwrap();
    let status = gm
        .child
        .wait()
        .unwrap_or_else(|err| panic!("wait gm: {err}"));
    assert!(status.success(), "gm release exited {status:?}");
}

#[then(expr = "a coding agent can connect at that MCP endpoint")]
async fn a_coding_agent_can_connect_at_that_mcp_endpoint(world: &mut World) {
    let http = port(world);
    host::poll_until("the agent to connect", || try_connect_gm(http)).await;
}

#[when(expr = "spook run listens with no MCP flag")]
async fn spook_run_listens_with_no_mcp_flag(world: &mut World) {
    let pack = host::pack(world).to_path_buf();
    host::listen(world, &pack).await;
    world.mark = usize::from(host::live_mut(world).mcp_port);
}

#[then(expr = "the reported MCP address serves \\/mcp")]
fn the_reported_mcp_addr_answers(world: &mut World) {
    let http = u16::try_from(world.mark).unwrap_or_else(|_| panic!("no MCP port"));
    let raw = post_initialize(http, "127.0.0.1", None);
    assert!(
        raw.lines().next().unwrap_or("").contains(" 200 "),
        "no /mcp on {http}\n{raw}"
    );
    assert!(raw.contains("\"protocolVersion\""), "{raw}");
}

#[when(expr = "an HTTP request arrives with an unknown MCP-Protocol-Version")]
async fn an_http_request_arrives_with_an_unknown_version(world: &mut World) {
    let (_tcp, http) = listen_on_free_ports(world).await;
    world.scratch = post_initialize_with_version(http, "1999-01-01");
}

#[when(expr = "an HTTP request arrives with the negotiated MCP-Protocol-Version")]
async fn an_http_request_arrives_with_the_negotiated_version(world: &mut World) {
    let (_tcp, http) = listen_on_free_ports(world).await;
    world.scratch = post_initialize_with_version(http, ProtocolVersion::LATEST.as_str());
}

#[then(expr = "the response status is {int}")]
fn the_response_status_is(world: &mut World, status: usize) {
    let line = world
        .scratch
        .lines()
        .next()
        .unwrap_or_else(|| panic!("no HTTP status\n{}", world.scratch));
    let needle = format!(" {status} ");
    assert!(line.contains(&needle), "status: {line}\n{}", world.scratch);
}

#[then(expr = "the response is accepted")]
fn the_response_is_accepted(world: &mut World) {
    let status = world
        .scratch
        .lines()
        .next()
        .unwrap_or_else(|| panic!("no HTTP status\n{}", world.scratch));
    assert!(
        status.contains(" 200 "),
        "status: {status}\n{}",
        world.scratch
    );
    assert!(
        world.scratch.contains("\"protocolVersion\""),
        "no initialize result\n{}",
        world.scratch
    );
}

fn port(world: &World) -> u16 {
    u16::try_from(world.mark).unwrap_or_else(|_| panic!("no MCP port"))
}

#[given(expr = "a docks table with a seated player and an open MCP endpoint")]
async fn a_docks_table_with_a_seated_player_and_an_open_mcp_endpoint(world: &mut World) {
    world.pack = Some(support::copy_pack("docks", "http-say"));
    let (_tcp, http) = listen_on_free_ports(world).await;
    let pack = host::pack(world).to_path_buf();
    host::seat_rowan(world, &pack).await;
    world.mark = usize::from(http);
}

#[when(expr = "a GM holds the seat over HTTP")]
async fn gm_holds_over_http(world: &mut World) {
    let (_tcp, http) = listen_on_free_ports(world).await;
    world.mark = usize::from(http);
    let mut holder = GmClient::connect(to_addr(http))
        .await
        .unwrap_or_else(|err| panic!("HTTP GM: {err}"));
    let status = call(&mut holder, "status", json!({})).await;
    assert_not_refused(&status, "status");
    world.http_gm = Some(holder);
}

#[then(expr = "the GM over HTTP can still call status")]
async fn the_http_gm_can_still_call_status(world: &mut World) {
    let holder = world
        .http_gm
        .as_mut()
        .unwrap_or_else(|| panic!("no GM holds the seat"));
    let status = call(holder, "status", json!({})).await;
    assert_not_refused(&status, "status");
}

#[when(expr = "an agent opens an MCP session")]
async fn an_agent_opens_an_mcp_session(world: &mut World) {
    if world.live.is_none() {
        let (_tcp, http) = listen_on_free_ports(world).await;
        world.mark = usize::from(http);
    }
    world.mcp_session = Some(RawSession::open(port(world)));
}

fn raw_session(world: &mut World) -> &mut RawSession {
    world
        .mcp_session
        .as_mut()
        .unwrap_or_else(|| panic!("no MCP session"))
}

#[then(expr = "the session opened")]
fn the_session_opened(world: &mut World) {
    let opened = &raw_session(world).opened;
    let status = opened.lines().next().unwrap_or_default();
    assert!(status.contains(" 200 "), "status: {status}\n{opened}");
    assert!(opened.contains("\"protocolVersion\""), "{opened}");
    assert!(!opened.contains("TABLE_FULL"), "{opened}");
}

#[when(expr = "that session opens its event stream")]
fn that_session_opens_its_event_stream(world: &mut World) {
    world.scratch = raw_session(world).open_event_stream();
}

#[then(expr = "the event stream opens with the hello")]
fn the_event_stream_opens_with_the_hello(world: &mut World) {
    let event = crate::mcp_client::read_event(&world.scratch);
    assert_eq!(
        event["params"]["body"]["kind"], "hello",
        "{}",
        world.scratch
    );
}

#[then(expr = "the event stream is refused with {string}")]
fn the_event_stream_is_refused_with(world: &mut World, code: String) {
    let event = crate::mcp_client::read_event(&world.scratch);
    let body = &event["params"]["body"];
    assert_eq!(body["kind"], "refused", "{}", world.scratch);
    assert_eq!(body["code"], code.as_str(), "{}", world.scratch);
}

#[then(expr = "that session's status call is refused with {string}")]
fn that_sessions_status_call_is_refused_with(world: &mut World, code: String) {
    let reply = raw_session(world).call("status");
    let needle = format!("\"kind\":\"refused\",\"code\":\"{code}\"");
    assert!(reply.contains("structuredContent"), "{reply}");
    assert!(reply.contains(&needle), "{reply}");
}

#[then(expr = "that session's status call is answered")]
fn that_sessions_status_call_is_answered(world: &mut World) {
    let reply = raw_session(world).call("status");
    assert!(reply.contains("structuredContent"), "{reply}");
    assert!(!reply.contains("\"kind\":\"refused\""), "{reply}");
}

#[when(expr = "the GM over HTTP leaves")]
async fn the_http_gm_leaves(world: &mut World) {
    let holder = world
        .http_gm
        .take()
        .unwrap_or_else(|| panic!("no GM holds the seat"));
    holder.close().await;
}

#[then(expr = "a GM can connect over HTTP")]
async fn a_gm_can_connect_over_http(world: &mut World) {
    if let Err(err) = try_connect_gm(port(world)).await {
        panic!("no GM could connect over HTTP: {err}");
    }
}

async fn try_connect_gm(port: u16) -> Result<(), String> {
    let mut gm = GmClient::connect(to_addr(port)).await?;
    let status = call(&mut gm, "status", json!({})).await;
    assert_not_refused(&status, "status");
    gm.close().await;
    Ok(())
}

#[when(expr = "the GM lists the tools")]
async fn the_gm_lists_the_tools(world: &mut World) {
    let holder = world
        .http_gm
        .as_ref()
        .unwrap_or_else(|| panic!("no GM holds the seat"));
    let tools = holder
        .list_tools()
        .await
        .unwrap_or_else(|err| panic!("tools/list: {err}"));
    world.scratch = serde_json::to_string(&tools).unwrap_or_else(|err| panic!("tools: {err}"));
}

/// One row per tool, in the order `tools/list` gives. Each input schema must refuse keys it
/// does not name, so its fields are all a GM may send.
#[then(expr = "the tools are the GM commands, and each input schema names these fields")]
fn the_tools_are_the_gm_commands(world: &mut World, step: &Step) {
    let tools: Vec<Value> =
        serde_json::from_str(&world.scratch).unwrap_or_else(|err| panic!("tools: {err}"));
    let mut rows = Vec::new();
    for tool in &tools {
        let tool_id = tool["name"].as_str().unwrap_or_default();
        let schema = &tool["inputSchema"];
        let closed = schema["additionalProperties"] == false;
        assert!(closed, "{tool_id} takes keys it does not name: {schema}");
        let fields = schema["properties"]
            .as_object()
            .into_iter()
            .flat_map(Map::keys);
        let required = schema["required"].as_array().into_iter().flatten();
        rows.push([
            tool_id.to_string(),
            sort_words(fields.map(String::as_str)),
            sort_words(required.filter_map(Value::as_str)),
        ]);
    }
    let table = step
        .table
        .as_ref()
        .unwrap_or_else(|| panic!("no tool table"));
    let want: Vec<_> = table.rows[1..]
        .iter()
        .map(|row| {
            [
                row[0].clone(),
                sort_words(row[1].split_whitespace()),
                sort_words(row[2].split_whitespace()),
            ]
        })
        .collect();
    assert_eq!(rows, want);
}

fn sort_words<'a>(words: impl Iterator<Item = &'a str>) -> String {
    let mut words: Vec<_> = words.collect();
    words.sort_unstable();
    words.join(" ")
}

#[then(regex = r#"^a queue call with "actor", "verb" and "to" is accepted$"#)]
async fn a_queue_call_with_to_is_accepted(world: &mut World) {
    let holder = world
        .http_gm
        .as_mut()
        .unwrap_or_else(|| panic!("no GM holds the seat"));
    let arguments = json!({"actor": "rowan", "verb": "wait", "to": ["wren"]});
    let queued = call(holder, "queue", arguments).await;
    assert_not_refused(&queued, "queue");
}

#[then(expr = "world time moved forward")]
fn world_time_moved_forward(world: &mut World) {
    let now = world_of(host::pack(world));
    assert!(now.world_time > 1080, "world_time {}", now.world_time);
}

#[when(expr = "a coding agent says good evening over HTTP")]
async fn agent_says_over_http(world: &mut World) {
    let http = port(world);
    let mark = host::player_mark(world);
    let mut gm = GmClient::connect(to_addr(http))
        .await
        .unwrap_or_else(|err| panic!("HTTP: {err}"));
    let speech = call(
        &mut gm,
        "say",
        json!({"to": "table", "text": "good evening"}),
    )
    .await;
    assert_not_refused(&speech, "say");
    world.player_added = host::wait_player_from(world, mark, "{\"kind\":\"say\"").await;
    world.http_gm = Some(gm);
}

#[when(expr = "the agent calls an unknown tool over HTTP")]
async fn agent_calls_an_unknown_tool(world: &mut World) {
    let mut gm = world
        .http_gm
        .take()
        .unwrap_or_else(|| panic!("no agent holds the seat"));
    let reply = call(&mut gm, "not_in_spec", json!({})).await;
    gm.close().await;
    world.scratch = json!(reply).to_string();
}

#[then(expr = "the unknown tool call was refused SCHEMA and the save is unchanged")]
fn the_unknown_tool_call_was_refused(world: &mut World) {
    let reply: Vec<ServerMessage> =
        serde_json::from_str(&world.scratch).unwrap_or_else(|err| panic!("reply: {err}"));
    let last = reply.last().map(|reply| &reply.body);
    let Some(ServerBody::Refused(refused)) = last else {
        panic!("not refused: {}", world.scratch);
    };
    assert_eq!(refused.code.as_str(), "SCHEMA", "{}", world.scratch);
    let saved = std::fs::read(host::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save: {err}"));
    assert_eq!(saved, host::live_mut(world).opened_save);
}

async fn call_each_gm_tool(gm: &mut GmClient) {
    for (tool_id, arguments) in [
        ("status", json!({})),
        ("perceive", json!({"as": "rowan"})),
        ("queue", json!({"actor": "rowan", "verb": "wait"})),
        ("say", json!({"to": "table", "text": "good evening"})),
        ("commit", json!({"include_round": true})),
    ] {
        let result = call(gm, tool_id, arguments).await;
        assert_not_refused(&result, tool_id);
    }
}

/// One tool call; every message it sent the seat, the reply last.
async fn call(gm: &mut GmClient, command: &str, arguments: Value) -> Vec<ServerMessage> {
    gm.call(command, &arguments)
        .await
        .unwrap_or_else(|err| panic!("{command}: {err}"))
}

/// The reply, the last message in the result, is not a refusal.
fn assert_not_refused(messages: &[ServerMessage], command: &str) {
    let reply = messages
        .last()
        .unwrap_or_else(|| panic!("{command} has no reply"));
    assert!(
        !matches!(reply.body, ServerBody::Refused(_)),
        "{command} was refused: {messages:?}"
    );
}

fn to_addr(port: u16) -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], port))
}

async fn listen_on_free_ports(world: &mut World) -> (u16, u16) {
    let pack = host::pack(world).to_path_buf();
    host::run_with_flags(
        world,
        env!("CARGO_TARGET_TMPDIR").as_ref(),
        &pack,
        &["--addr", "127.0.0.1:0", "--mcp-addr", "127.0.0.1:0"],
    )
    .await
}

fn post_initialize_with_version(port: u16, version: &str) -> String {
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{version}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"probe\",\"version\":\"0\"}}}}}}"
    );
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nMCP-Protocol-Version: {version}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|err| panic!("connect HTTP: {err}"));
    stream
        .set_read_timeout(Some(support::WAIT_LIMIT))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|err| panic!("write HTTP: {err}"));
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    buf
}

pub(crate) fn post_initialize(port: u16, host: &str, origin: Option<&str>) -> String {
    let version = ProtocolVersion::LATEST.as_str();
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{version}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"probe\",\"version\":\"0\"}}}}}}"
    );
    let origin = match origin {
        Some(origin) => format!("Origin: {origin}\r\n"),
        None => String::new(),
    };
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}:{port}\r\n{origin}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|err| panic!("connect HTTP: {err}"));
    stream
        .set_read_timeout(Some(support::WAIT_LIMIT))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|err| panic!("write HTTP: {err}"));
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    buf
}

#[given(expr = "an AI GM on \\/mcp and a plain GM client on WebSocket")]
async fn an_mcp_gm_and_a_websocket_gm(world: &mut World) {
    let pack = support::copy_pack("docks", "parity-mcp");
    world.pack = Some(pack.clone());
    host::listen(world, &pack).await;
    host::seat_gm(world).await;
    let ws_pack = support::copy_pack("docks", "parity-ws");
    let flags = ["--addr", "127.0.0.1:0"];
    let running = host::wait_addrs(host::spawn(
        env!("CARGO_TARGET_TMPDIR").as_ref(),
        &ws_pack,
        &flags,
    ))
    .await;
    let gm = host::Seat::open(running.game.port(), "gm").await;
    host::wait_suffix(&gm.text, 0, "\"kind\":\"hello\"").await;
    host::live_mut(world).ws_gm = Some(gm);
    world.ws_host = Some((running, ws_pack));
}

/// Bram stands in the street. Travel needs a target, and the shop is next to it.
const TRAVEL_THEN_COMMIT: [(&str, &str); 3] = [
    ("queue", r#"{"actor":"bram","verb":"travel"}"#),
    (
        "queue",
        r#"{"actor":"bram","verb":"travel","target":"docks_shop.docks_street"}"#,
    ),
    ("commit", r#"{"include_round":true}"#),
];

#[when(expr = "each sends travel for an unheld npc and commits")]
async fn each_sends_travel_and_commits(world: &mut World) {
    let mut over_mcp = Vec::new();
    let mut over_ws = Vec::new();
    for (command, args) in TRAVEL_THEN_COMMIT {
        let args: Value = serde_json::from_str(args).unwrap_or_else(|err| panic!("{err}"));
        let live = host::live_mut(world);
        let gm = live.gm.as_mut().unwrap_or_else(|| panic!("no GM on /mcp"));
        let messages = gm
            .call(command, &args)
            .await
            .unwrap_or_else(|err| panic!("{command} over /mcp: {err}"));
        over_mcp.push(messages.iter().map(|message| json!(message)).collect());
        let gm = live
            .ws_gm
            .as_ref()
            .unwrap_or_else(|| panic!("no GM on WebSocket"));
        let mark = gm.mark();
        let id = gm.send(command, args);
        gm.wait_reply(&id).await;
        let text = support::text_of(&gm.text);
        over_ws.push(
            text[mark..]
                .lines()
                .map(|line| serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}")))
                .collect(),
        );
    }
    world.seen = vec![
        ("/mcp".to_string(), lines_of(over_mcp)),
        ("WebSocket".to_string(), lines_of(over_ws)),
    ];
}

/// Each call's messages as one line each, its reply last. The reply's `reply_to` and an
/// intent's id are the client's and the host's own, so they are blanked.
fn lines_of(calls: Vec<Vec<Value>>) -> Vec<String> {
    calls
        .into_iter()
        .map(|mut messages| {
            for message in &mut messages {
                if let Some(intent) = message["body"].get_mut("intent") {
                    *intent = json!("<intent>");
                }
            }
            let reply = messages
                .last_mut()
                .and_then(|reply| reply.as_object_mut())
                .unwrap_or_else(|| panic!("a call got no reply"));
            assert!(reply.remove("reply_to").is_some(), "{reply:?}");
            json!(messages).to_string()
        })
        .collect()
}

#[then(expr = "both see the same messages and the same refusals")]
fn both_see_the_same_messages(world: &mut World) {
    let [(_, mcp), (_, ws)] = world.seen.as_slice() else {
        panic!("two GMs did not both send");
    };
    assert_eq!(mcp, ws);
    let reply_kinds: Vec<Value> = mcp
        .iter()
        .map(|call| {
            let messages: Vec<Value> = serde_json::from_str(call).unwrap();
            messages.last().unwrap()["body"]["kind"].clone()
        })
        .collect();
    assert_eq!(
        reply_kinds,
        [json!("refused"), json!("queued"), json!("committed")]
    );
    assert!(mcp[2].contains("\"kind\":\"round\""), "{}", mcp[2]);
}
