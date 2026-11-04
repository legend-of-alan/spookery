//! `docs/seat-protocol.md` is a capture of one live docks table.
//! Each fenced example is named. This test fails when a name is missing,
//! extra, or different from the table.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crate::mcp_client::{event_line, GmClient};
use rmcp::model::ProtocolVersion;
use serde_json::{json, Value};
use spookery::protocol::{Addrs, ClientMessage, ServerBody, ServerMessage, ToolResult};
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

use crate::support;

/// A seat that never answers still has to fail the test. The line itself
/// unblocks the wait.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

const SCRUBBED_GAME_ADDR: &str = "127.0.0.1:<game>";
const SCRUBBED_MCP_ADDR: &str = "127.0.0.1:<mcp>";
const SCRUBBED_TOKEN: &str = "<token>";

enum Example {
    Json(Value),
    Text(String),
}

/// One player connection on `/player` on the game port. A reader thread hands over
/// each message.
struct PlayerClient {
    write: WebSocket<TcpStream>,
    lines: Receiver<String>,
    next_id: u64,
}

struct Table {
    child: Child,
    _stdin: ChildStdin,
    pack: PathBuf,
    game: u16,
    mcp_port: u16,
    /// The `listening` line `spook run` printed.
    addrs_line: String,
}

impl Drop for Table {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.pack);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_seat_page_matches_a_live_table() {
    let captured = capture();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/seat-protocol.md");
    let doc = std::fs::read_to_string(&path).unwrap_or_default();
    let documented = examples_in(&doc);
    let mut problems = Vec::new();
    for name in captured.keys() {
        if !documented.contains_key(name) {
            problems.push(format!("missing example `{name}`"));
        }
    }
    for name in documented.keys() {
        if !captured.contains_key(name) {
            problems.push(format!("doc example `{name}` was not captured"));
        }
    }
    for (name, live) in &captured {
        let Some(doc_example) = documented.get(name) else {
            continue;
        };
        if let Some(problem) = differ(name, live, doc_example) {
            problems.push(problem);
        }
    }
    if problems.is_empty() {
        return;
    }
    let dump = std::env::temp_dir().join("seat-protocol-capture.md");
    std::fs::write(&dump, render(&captured)).unwrap_or_else(|err| panic!("write capture: {err}"));
    panic!(
        "docs/seat-protocol.md drifted ({}):\n{}\ncaptures: {}",
        problems.len(),
        problems.join("\n"),
        dump.display()
    );
}

fn capture() -> BTreeMap<String, Example> {
    let mut out = BTreeMap::new();
    let table = listen();
    let mut player = open_player(table.game);

    let addrs = serde_json::from_str(&table.addrs_line)
        .unwrap_or_else(|err| panic!("addrs: {err}\n{}", table.addrs_line));
    insert_json(&mut out, "listening", addrs);
    insert_json(&mut out, "player-hello", next_json(&player));
    let claim = send(&mut player, "claim", json!({"actor": "pc_rowan"}));
    insert_json(&mut out, "player-claim", claim);
    let mut claimed = next_json(&player);
    assert_eq!(claimed["body"]["kind"], "claimed", "{claimed}");
    let token = scrub_token(&mut claimed["body"]);
    insert_json(&mut out, "player-claimed", claimed);
    let resume = next_json(&player);
    assert_eq!(resume["body"]["kind"], "resume", "{resume}");
    insert_json(&mut out, "player-resume", resume);
    let perceive = next_json(&player);
    assert_eq!(perceive["body"]["kind"], "view", "{perceive}");
    insert_json(&mut out, "player-perceive", perceive);

    insert_json(
        &mut out,
        "player-look",
        send(&mut player, "look", json!({})),
    );
    let looked = next_json(&player);
    assert_eq!(looked["body"]["kind"], "view", "{looked}");
    insert_json(&mut out, "player-look-view", looked);

    insert_json(
        &mut out,
        "player-seat-taken",
        refused_reply(table.game, "claim", json!({"actor": "pc_rowan"})),
    );
    insert_json(
        &mut out,
        "player-unknown-pc",
        refused_reply(table.game, "claim", json!({"actor": "no_such"})),
    );
    insert_json(
        &mut out,
        "player-choose-pc",
        refused_reply(table.game, "wait", json!({})),
    );
    insert_json(
        &mut out,
        "player-line-too-long",
        send_too_long_message(table.game),
    );

    let version = ProtocolVersion::LATEST.as_str();
    let initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": { "name": "seat-protocol", "version": "0" }
        }
    });
    insert_json(&mut out, "initialize-request", initialize.clone());
    let (opened, session) = post_initialize(table.mcp_port, &initialize);
    insert_json(&mut out, "initialize-result", opened.clone());
    let negotiated = opened["result"]["protocolVersion"]
        .as_str()
        .unwrap_or_else(|| panic!("no protocol version\n{opened}"))
        .to_string();
    // DELETE is a legacy-lifecycle call. The newest protocol version is refused
    // with 405, so the header is the version the handshake returned.
    delete_session(table.mcp_port, &session, &negotiated);

    let mcp_addr = std::net::SocketAddr::from(([127, 0, 0, 1], table.mcp_port));
    let mut gm = support::block_on(GmClient::connect(mcp_addr))
        .unwrap_or_else(|err| panic!("seat GM: {err}"));

    let status_args = serde_json::json!({});
    insert_json(
        &mut out,
        "status-request",
        tool_call(1, "status", &status_args),
    );
    insert_json(
        &mut out,
        "status-result",
        call(&mut gm, "status", status_args),
    );

    let perceive_args = serde_json::json!({"as": "pc_rowan"});
    insert_json(
        &mut out,
        "perceive-request",
        tool_call(2, "perceive", &perceive_args),
    );
    let gm_perceive = call(&mut gm, "perceive", perceive_args);
    assert_eq!(
        &gm_perceive["messages"][0]["body"],
        &out_json(&out, "player-perceive")["body"],
        "GM perceive diverged from the player welcome"
    );

    let say = send(
        &mut player,
        "say",
        json!({"address": "table", "text": "evening"}),
    );
    insert_json(&mut out, "player-say", say);
    let said = next_json(&player);
    assert_eq!(said["body"]["kind"], "say", "{said}");
    insert_json(&mut out, "player-say-frame", said);
    insert_json(&mut out, "say-event", one_event(&mut gm));

    let say_args = serde_json::json!({"address": "table", "text": "good evening"});
    insert_json(&mut out, "say-request", tool_call(3, "say", &say_args));
    insert_json(&mut out, "say-result", call(&mut gm, "say", say_args));
    let heard = next_json(&player);
    assert_eq!(heard["body"]["kind"], "say", "{heard}");
    insert_json(&mut out, "player-hears-gm", heard);
    let held_args = serde_json::json!({"actor_id": "pc_rowan", "move": "wait"});
    insert_json(
        &mut out,
        "intent-held-request",
        tool_call(4, "queue", &held_args),
    );
    insert_json(&mut out, "intent-held", call(&mut gm, "queue", held_args));

    let intent_args = serde_json::json!({
        "actor_id": "aldric",
        "move": "wait",
        "id": "gm-wait"
    });
    insert_json(
        &mut out,
        "intent-request",
        tool_call(5, "queue", &intent_args),
    );
    insert_json(
        &mut out,
        "intent-result",
        call(&mut gm, "queue", intent_args),
    );

    insert_json(
        &mut out,
        "player-grit",
        send(&mut player, "grit", json!({})),
    );
    let queued = next_json(&player);
    assert_eq!(queued["body"]["kind"], "queued", "{queued}");
    insert_json(&mut out, "player-queued", queued);
    insert_json(&mut out, "queued-event", one_event(&mut gm));

    let refuse_commit = serde_json::json!({"include_round": false});
    insert_json(
        &mut out,
        "commit-without-round-request",
        tool_call(6, "commit", &refuse_commit),
    );
    insert_json(
        &mut out,
        "commit-without-round",
        call(&mut gm, "commit", refuse_commit),
    );

    let commit_args = serde_json::json!({"include_round": true});
    insert_json(
        &mut out,
        "commit-request",
        tool_call(7, "commit", &commit_args),
    );
    insert_json(
        &mut out,
        "commit-result",
        call(&mut gm, "commit", commit_args),
    );
    let round = next_json(&player);
    assert_eq!(round["body"]["kind"], "round", "{round}");
    insert_json(&mut out, "player-round", round);
    let check = next_json(&player);
    assert_eq!(check["body"]["kind"], "check", "{check}");
    insert_json(&mut out, "player-check", check);
    let again = next_json(&player);
    assert_eq!(again["body"]["kind"], "resume", "{again}");
    let refreshed = next_json(&player);
    assert_eq!(refreshed["body"]["kind"], "view", "{refreshed}");

    // A second round moves an item the player can see, so the change line is not empty.
    send(&mut player, "take", json!({"target": "lamp_oil"}));
    let queued_take = next_json(&player);
    assert_eq!(queued_take["body"]["kind"], "queued", "{queued_take}");
    let _ = one_event(&mut gm);
    call(
        &mut gm,
        "commit",
        serde_json::json!({"include_round": true}),
    );
    let taken_round = next_json(&player);
    assert_eq!(taken_round["body"]["kind"], "round", "{taken_round}");
    let taken = next_json(&player);
    assert_eq!(taken["body"]["kind"], "resume", "{taken}");
    let after_take = next_json(&player);
    assert_eq!(after_take["body"]["kind"], "view", "{after_take}");
    let changes = next_json(&player);
    assert_eq!(changes["body"]["kind"], "lines", "{changes}");
    insert_json(&mut out, "player-changes", changes);

    let unknown = send(&mut player, "blah", json!({}));
    insert_json(&mut out, "player-unknown-word", unknown);
    insert_json(&mut out, "player-word-schema", next_json(&player));

    insert_json(
        &mut out,
        "player-empty-say",
        send(&mut player, "say", json!({})),
    );
    insert_json(&mut out, "player-schema", next_json(&player));

    let locked = send(&mut player, "travel", json!({"target": "stairs"}));
    insert_json(&mut out, "player-locked-exit", locked);
    insert_json(&mut out, "player-no-exit", next_json(&player));

    let bad_tool = serde_json::json!({});
    insert_json(
        &mut out,
        "unknown-tool-request",
        tool_call(9, "not_in_spec", &bad_tool),
    );
    insert_json(
        &mut out,
        "unknown-tool",
        call(&mut gm, "not_in_spec", bad_tool),
    );

    let unscoped = serde_json::json!({});
    insert_json(
        &mut out,
        "unscoped-perceive-request",
        tool_call(10, "perceive", &unscoped),
    );
    insert_json(
        &mut out,
        "unscoped-perceive",
        call(&mut gm, "perceive", unscoped),
    );

    let mouse = connect_player(table.game, "pc_harbormouse");
    let wren = connect_player(table.game, "pc_wren");
    // The claim's reply means that claim has the seat. The fourth claim comes after both.
    assert_eq!(next_json(&mouse)["body"]["kind"], "claimed");
    assert_eq!(next_json(&wren)["body"]["kind"], "claimed");
    insert_json(
        &mut out,
        "player-session-full",
        refused_reply(table.game, "claim", json!({"actor": "pc_extra"})),
    );

    insert_json(
        &mut out,
        "gm-session-full",
        post_initialize_when_full(table.mcp_port, &initialize),
    );

    // Rowan's connection drops without a release. The claim holds, and keeps the say.
    let last_seq = out_json(&out, "player-no-exit")["seq"].clone();
    player
        .write
        .get_ref()
        .shutdown(std::net::Shutdown::Both)
        .unwrap_or_else(|err| panic!("shutdown: {err}"));
    drop(player);
    call(
        &mut gm,
        "say",
        serde_json::json!({"address": "table", "text": "still there?"}),
    );
    let _ = gm.drain_events();
    let mut back = connect_player_unclaimed(table.game);
    let mut reclaim = send(
        &mut back,
        "claim",
        json!({"token": token, "last_seq": last_seq}),
    );
    scrub_token(&mut reclaim["body"]["args"]);
    insert_json(&mut out, "player-claim-token", reclaim);
    let mut reclaimed = next_json(&back);
    assert_eq!(reclaimed["body"]["kind"], "claimed", "{reclaimed}");
    scrub_token(&mut reclaimed["body"]);
    insert_json(&mut out, "player-claimed-token", reclaimed);
    let _ = next_json(&back);
    let _ = next_json(&back);
    send(&mut back, "release", json!({}));
    assert_eq!(next_json(&back)["body"]["kind"], "released");
    let again = connect_player(table.game, "pc_rowan");
    let mut replay = next_json(&again);
    assert_eq!(replay["body"]["kind"], "claimed", "{replay}");
    scrub_token(&mut replay["body"]);
    insert_json(&mut out, "player-replay-say", replay);

    let release_args = serde_json::json!({});
    insert_json(
        &mut out,
        "release-request",
        tool_call(12, "release", &release_args),
    );
    insert_json(
        &mut out,
        "release-result",
        call(&mut gm, "release", release_args),
    );
    out
}

fn listen() -> Table {
    let pack = crate::support::copy_docks("seat-protocol");
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command
        .arg("run")
        .arg(crate::support::pack_arg(&pack))
        .args(["--addr", "127.0.0.1:0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"));
    let stdin = child.stdin.take().unwrap_or_else(|| panic!("no stdin"));
    let mut stdout = child.stdout.take().unwrap_or_else(|| panic!("no stdout"));
    let stderr = child.stderr.take().unwrap_or_else(|| panic!("no stderr"));
    std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut buf = [0u8; 256];
        while stderr.read(&mut buf).unwrap_or(0) > 0 {}
    });
    let (game, mcp_port, addrs_line) = read_addrs(&mut stdout);
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        while stdout.read(&mut buf).unwrap_or(0) > 0 {}
    });
    Table {
        child,
        _stdin: stdin,
        pack,
        game,
        mcp_port,
        addrs_line,
    }
}

/// The first stdout line, which is the `listening` line: its two ports and the line.
fn read_addrs(stdout: &mut impl Read) -> (u16, u16, String) {
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match stdout.read(&mut buf) {
            Ok(0) => panic!("spook run closed stdout\n{}", String::from_utf8_lossy(&got)),
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if let Some(end) = got.iter().position(|byte| *byte == b'\n') {
                    let line = std::str::from_utf8(&got[..end])
                        .unwrap_or_else(|err| panic!("{err}"))
                        .trim_end_matches('\r');
                    let addrs: Addrs = serde_json::from_str(line)
                        .unwrap_or_else(|err| panic!("addrs: {err}\n{line}"));
                    let port = |addr: &str| {
                        loopback_port(addr).unwrap_or_else(|| panic!("not loopback\n{line}"))
                    };
                    return (port(&addrs.game), port(&addrs.mcp), line.into());
                }
            }
            Err(err) => panic!("stdout: {err}"),
        }
    }
}

/// Opens a player connection. The hello is its first message.
fn open_player(port: u16) -> PlayerClient {
    let stream = connect(port);
    let write = stream
        .try_clone()
        .unwrap_or_else(|err| panic!("clone: {err}"));
    let url = format!("ws://127.0.0.1:{port}/player");
    let (mut read, _) =
        tungstenite::client(url, stream).unwrap_or_else(|err| panic!("player seat: {err}"));
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(message) = read.read() {
            if let Message::Text(text) = message {
                if tx.send(text.to_string()).is_err() {
                    break;
                }
            }
        }
    });
    PlayerClient {
        write: WebSocket::from_raw_socket(write, Role::Client, None),
        lines,
        next_id: 1,
    }
}

/// Opens a player connection, reads its hello, and claims `actor`.
fn connect_player(port: u16, actor: &str) -> PlayerClient {
    let mut player = open_player(port);
    let hello = next_json(&player);
    assert_eq!(hello["body"]["kind"], "hello", "{hello}");
    send(&mut player, "claim", json!({"actor": actor}));
    player
}

/// Sends `word` with `args` under the next id. Returns the message sent.
fn send(player: &mut PlayerClient, word: &str, args: Value) -> Value {
    let id = format!("c{}", player.next_id);
    player.next_id += 1;
    let message = serde_json::to_value(ClientMessage::new(&id, word, args))
        .unwrap_or_else(|err| panic!("{word}: {err}"));
    player
        .write
        .send(Message::text(message.to_string()))
        .unwrap_or_else(|err| panic!("send: {err}"));
    message
}

fn next_line(player: &PlayerClient) -> String {
    player
        .lines
        .recv_timeout(READ_TIMEOUT)
        .unwrap_or_else(|err| panic!("no player message: {err}"))
}

fn next_json(player: &PlayerClient) -> Value {
    let line = next_line(player);
    serde_json::from_str(&line).unwrap_or_else(|err| panic!("{err} in {line}"))
}

fn connect(port: u16) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", port))
        .unwrap_or_else(|err| panic!("connect {port}: {err}"));
    stream
        .set_nodelay(true)
        .unwrap_or_else(|err| panic!("nodelay: {err}"));
    stream
}

/// A new player connection sends one word. Returns its reply.
fn refused_reply(port: u16, word: &str, args: Value) -> Value {
    let mut player = connect_player_unclaimed(port);
    send(&mut player, word, args);
    next_json(&player)
}

/// A new player connection sends one message past the cap. Returns its reply.
fn send_too_long_message(port: u16) -> Value {
    let mut player = connect_player_unclaimed(port);
    let long = json!({"id": "c1", "body": {"word": "wait"}, "pad": "x".repeat(8 * 1024)});
    player
        .write
        .send(Message::text(long.to_string()))
        .unwrap_or_else(|err| panic!("send: {err}"));
    next_json(&player)
}

/// A player connection past its hello, with no claim.
fn connect_player_unclaimed(port: u16) -> PlayerClient {
    let player = open_player(port);
    let hello = next_json(&player);
    assert_eq!(hello["body"]["kind"], "hello", "{hello}");
    player
}

/// One tool call. Returns its structured content.
fn call(gm: &mut GmClient, word: &str, arguments: Value) -> Value {
    let messages =
        support::block_on(gm.call(word, &arguments)).unwrap_or_else(|err| panic!("{word}: {err}"));
    json!(ToolResult { messages })
}

/// The next GM event after the hello.
fn one_event(gm: &mut GmClient) -> Value {
    let is_hello = |event: &ServerMessage| matches!(event.body, ServerBody::Hello { .. });
    let mut ready = gm.drain_events();
    ready.retain(|event| !is_hello(event));
    while ready.is_empty() {
        match support::block_on(gm.recv()) {
            Some(Ok(event)) if is_hello(&event) => {}
            Some(Ok(event)) => ready.push(event),
            other => panic!("expected a GM event, got {other:?}"),
        }
    }
    assert_eq!(ready.len(), 1, "GM events: {ready:?}");
    let line = event_line(&ready[0]);
    serde_json::from_str(&line).unwrap_or_else(|err| panic!("{err} in {line}"))
}

fn tool_call(id: u64, name: &str, arguments: &Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
}

fn post_initialize(port: u16, body: &Value) -> (Value, String) {
    let raw = send_http(
        port,
        "POST",
        &serde_json::to_string(body).unwrap(),
        None,
        ProtocolVersion::LATEST.as_str(),
    );
    let session = header(&raw, "mcp-session-id").unwrap_or_else(|| panic!("no session\n{raw}"));
    let value = json_rpc(&raw).unwrap_or_else(|| panic!("no initialize result\n{raw}"));
    (value, session)
}

fn post_initialize_when_full(port: u16, body: &Value) -> Value {
    let raw = send_http(
        port,
        "POST",
        &serde_json::to_string(body).unwrap(),
        None,
        ProtocolVersion::LATEST.as_str(),
    );
    json_rpc(&raw).unwrap_or_else(|| panic!("no refusal\n{raw}"))
}

fn delete_session(port: u16, session: &str, version: &str) {
    let raw = send_http(port, "DELETE", "", Some(session), version);
    let status = raw.lines().next().unwrap_or("");
    assert!(
        status.contains(" 200 ") || status.contains(" 202 ") || status.contains(" 204 "),
        "DELETE did not release the seat\n{raw}"
    );
}

fn send_http(port: u16, method: &str, body: &str, session: Option<&str>, version: &str) -> String {
    let session = session
        .map(|id| format!("mcp-session-id: {id}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{session}MCP-Protocol-Version: {version}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = connect(port);
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|err| panic!("write HTTP: {err}"));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        if is_http_complete(&buf) {
            break;
        }
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => break,
            Err(err) => panic!("read HTTP: {err}"),
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn is_http_complete(buf: &[u8]) -> bool {
    let text = String::from_utf8_lossy(buf);
    let Some(split) = text.find("\r\n\r\n") else {
        return false;
    };
    let head = &text[..split];
    let body = &text[split + 4..];
    if let Some(len) = content_length(head) {
        return body.len() >= len;
    }
    let decoded = decode_body(head, body);
    decoded.contains("\"jsonrpc\"")
        && (decoded.ends_with("\n\n") || decoded.ends_with("\r\n\r\n") || decoded.ends_with('}'))
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn header(raw: &str, name: &str) -> Option<String> {
    let head = raw.split("\r\n\r\n").next().unwrap_or(raw);
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.eq_ignore_ascii_case(name) {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

fn json_rpc(raw: &str) -> Option<Value> {
    let head_end = raw.find("\r\n\r\n").map(|at| at + 4).unwrap_or(0);
    let head = &raw[..head_end.saturating_sub(4)];
    let body = decode_body(head, &raw[head_end..]);
    let bytes = body.as_bytes();
    let mut start = 0;
    while let Some(rel) = bytes[start..].iter().position(|byte| *byte == b'{') {
        let at = start + rel;
        let mut parsed = serde_json::Deserializer::from_slice(&bytes[at..]).into_iter::<Value>();
        if let Some(Ok(value)) = parsed.next() {
            if value.get("jsonrpc").is_some() {
                return Some(value);
            }
        }
        start = at + 1;
    }
    None
}

fn decode_body(head: &str, body: &str) -> String {
    let chunked = head.lines().any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
    });
    if !chunked {
        return body.to_string();
    }
    let mut rest = body;
    let mut out = String::new();
    while let Some((size, after)) = rest.split_once("\r\n") {
        let Ok(len) = usize::from_str_radix(size.trim(), 16) else {
            break;
        };
        if len == 0 {
            break;
        }
        if after.len() < len {
            break;
        }
        out.push_str(&after[..len]);
        rest = &after[len..];
        if let Some(stripped) = rest.strip_prefix("\r\n") {
            rest = stripped;
        }
    }
    out
}

fn insert_json(out: &mut BTreeMap<String, Example>, name: &str, value: Value) {
    out.insert(name.to_string(), Example::Json(value));
}

fn out_json<'a>(out: &'a BTreeMap<String, Example>, name: &str) -> &'a Value {
    match out.get(name) {
        Some(Example::Json(value)) => value,
        _ => panic!("no json example {name}"),
    }
}

fn differ(name: &str, live: &Example, documented: &Example) -> Option<String> {
    match (live, documented) {
        (Example::Text(live), Example::Text(doc)) if live == doc => None,
        (Example::Text(live), Example::Text(doc)) => {
            Some(format!("`{name}` text\n  live: {live}\n  doc:  {doc}"))
        }
        (Example::Json(live), Example::Json(doc)) => {
            let mut live = live.clone();
            let mut doc = doc.clone();
            scrub(&mut live);
            scrub(&mut doc);
            if live == doc {
                None
            } else {
                Some(format!("`{name}` json\n  live: {live}\n  doc:  {doc}"))
            }
        }
        _ => Some(format!("`{name}` is not the same kind of example")),
    }
}

fn scrub(value: &mut Value) {
    let Ok(Addrs { game, mcp }) = serde_json::from_value(value.clone()) else {
        return;
    };
    assert!(loopback_port(&game).is_some(), "{game}");
    assert!(loopback_port(&mcp).is_some(), "{mcp}");
    assert_ne!(game, mcp, "game and MCP ports");
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert(
        "game".to_string(),
        Value::String(SCRUBBED_GAME_ADDR.to_string()),
    );
    object.insert(
        "mcp".to_string(),
        Value::String(SCRUBBED_MCP_ADDR.to_string()),
    );
}

/// Swaps a claim's token for `<token>`, since each table mints its own, and returns it.
fn scrub_token(body: &mut Value) -> String {
    let token = body["token"]
        .as_str()
        .unwrap_or_else(|| panic!("no token\n{body}"))
        .to_string();
    body["token"] = Value::String(SCRUBBED_TOKEN.to_string());
    token
}

fn loopback_port(text: &str) -> Option<u16> {
    text.strip_prefix("127.0.0.1:")?.parse().ok()
}

struct Fence {
    name: Option<String>,
    kind: String,
    body: String,
}

fn examples_in(doc: &str) -> BTreeMap<String, Example> {
    let mut out = BTreeMap::new();
    for fence in fences(doc) {
        let Some(name) = fence.name else {
            assert!(
                fence.kind != "json",
                "json fence has no example name:\n{}",
                fence.body
            );
            continue;
        };
        let example = match fence.kind.as_str() {
            "json" => Example::Json(
                serde_json::from_str(&fence.body)
                    .unwrap_or_else(|err| panic!("example `{name}`: {err}\n{}", fence.body)),
            ),
            "text" => Example::Text(fence.body.trim_end().to_string()),
            other => panic!("example `{name}` uses `{other}`, not json or text"),
        };
        assert!(
            out.insert(name.clone(), example).is_none(),
            "duplicate example `{name}`"
        );
    }
    out
}

fn fences(doc: &str) -> Vec<Fence> {
    let mut out = Vec::new();
    let mut pending = None;
    let mut lines = doc.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(name) = line
            .strip_prefix("<!-- example: ")
            .and_then(|rest| rest.strip_suffix(" -->"))
        {
            pending = Some(name.to_string());
            continue;
        }
        let Some(kind) = line.strip_prefix("```") else {
            continue;
        };
        if kind.starts_with('`') {
            continue;
        }
        let mut body = String::new();
        for body_line in lines.by_ref() {
            if body_line == "```" {
                break;
            }
            body.push_str(body_line);
            body.push('\n');
        }
        out.push(Fence {
            name: pending.take(),
            kind: kind.to_string(),
            body,
        });
    }
    out
}

fn render(captured: &BTreeMap<String, Example>) -> String {
    let mut out = String::new();
    for (name, example) in captured {
        out.push_str(&format!("<!-- example: {name} -->\n"));
        match example {
            Example::Json(value) => {
                out.push_str("```json\n");
                out.push_str(
                    &serde_json::to_string_pretty(value).unwrap_or_else(|err| panic!("{err}")),
                );
                out.push('\n');
            }
            Example::Text(text) => {
                out.push_str("```text\n");
                out.push_str(text);
                out.push('\n');
            }
        }
        out.push_str("```\n\n");
    }
    out
}
