//! `docs/seat-protocol.md` is a capture of one live docks table.
//! Each fenced example is named. This test fails when a name is missing,
//! extra, or different from the table.

use std::collections::BTreeMap;
use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crate::mcp_client::{event_line, read_event, GmClient, RawSession};
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

/// One connection to `/player` or `/gm` on the game port. A reader thread hands over each
/// message.
struct SeatClient {
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
    let addrs = serde_json::from_str(&table.addrs_line)
        .unwrap_or_else(|err| panic!("addrs: {err}\n{}", table.addrs_line));
    insert_json(&mut out, "listening", addrs);

    let mut gm = open_seat(table.game, "gm");
    insert_json(&mut out, "gm-hello", next_kind(&gm, "hello"));
    let mut player = open_seat(table.game, "player");
    insert_json(&mut out, "player-hello", next_kind(&player, "hello"));
    let claim = send(&mut player, "claim", json!({"actor": "pc_rowan"}));
    insert_json(&mut out, "player-claim", claim);
    let mut claimed = next_kind(&player, "claimed");
    let token = scrub_token(&mut claimed["body"]);
    insert_json(&mut out, "player-claimed", claimed);
    insert_json(&mut out, "player-resume", next_kind(&player, "resume"));
    insert_json(&mut out, "player-view", next_kind(&player, "view"));
    insert_json(&mut out, "gm-presence", next_kind(&gm, "presence"));
    insert_json(
        &mut out,
        "player-choose-pc",
        refused_reply(table.game, "wait", json!({})),
    );

    let say = send(
        &mut player,
        "say",
        json!({"address": "table", "text": "evening"}),
    );
    insert_json(&mut out, "player-say", say);
    insert_json(&mut out, "player-said", next_kind(&player, "say"));
    insert_json(&mut out, "gm-heard", next_kind(&gm, "say"));

    // The save's stamp is the pack's own until a round commits, so status comes first.
    send(&mut gm, "status", json!({}));
    insert_json(&mut out, "gm-status", next_kind(&gm, "status"));
    let queue = send(
        &mut gm,
        "queue",
        json!({"actor_id": "aldric", "move": "wait", "id": "gm-wait"}),
    );
    insert_json(&mut out, "gm-queue", queue);
    insert_json(&mut out, "gm-queued", next_kind(&gm, "queued"));
    insert_json(
        &mut out,
        "player-grit",
        send(&mut player, "grit", json!({})),
    );
    insert_json(&mut out, "player-queued", next_kind(&player, "queued"));
    next_kind(&gm, "queued");

    let commit = send(&mut gm, "commit", json!({"include_round": true}));
    insert_json(&mut out, "gm-commit", commit);
    insert_json(&mut out, "gm-round", next_kind(&gm, "round"));
    insert_json(&mut out, "gm-committed", next_kind(&gm, "committed"));
    insert_json(&mut out, "player-round", next_kind(&player, "round"));
    insert_json(&mut out, "player-check", next_kind(&player, "check"));
    next_kind(&player, "resume");
    next_kind(&player, "view");

    // A second round moves an item the player can see, so the change line is not empty.
    send(&mut player, "take", json!({"target": "lamp_oil"}));
    next_kind(&player, "queued");
    next_kind(&gm, "queued");
    send(&mut gm, "commit", json!({"include_round": true}));
    next_kind(&gm, "round");
    next_kind(&gm, "committed");
    next_kind(&player, "round");
    next_kind(&player, "resume");
    next_kind(&player, "view");
    insert_json(&mut out, "player-lines", next_kind(&player, "lines"));

    send(&mut player, "travel", json!({"target": "stairs"}));
    insert_json(&mut out, "player-no-exit", next_kind(&player, "refused"));

    // Rowan's connection drops without a release. The claim holds, and keeps the say.
    let last_seq = out_json(&out, "player-no-exit")["seq"].clone();
    player
        .write
        .get_ref()
        .shutdown(std::net::Shutdown::Both)
        .unwrap_or_else(|err| panic!("shutdown: {err}"));
    drop(player);
    send(
        &mut gm,
        "say",
        json!({"address": "table", "text": "still there?"}),
    );
    next_kind(&gm, "say");
    let mut back = open_unclaimed(table.game);
    let mut reclaim = send(
        &mut back,
        "claim",
        json!({"token": token, "last_seq": last_seq}),
    );
    scrub_token(&mut reclaim["body"]["args"]);
    insert_json(&mut out, "player-claim-token", reclaim);
    let mut reclaimed = next_kind(&back, "claimed");
    scrub_token(&mut reclaimed["body"]);
    insert_json(&mut out, "player-claimed-token", reclaimed);
    next_kind(&back, "resume");
    next_kind(&back, "view");
    send(&mut gm, "release", json!({}));
    next_kind(&gm, "released");

    let mcp_addr = std::net::SocketAddr::from(([127, 0, 0, 1], table.mcp_port));
    let mut mcp = support::block_on(GmClient::connect(mcp_addr))
        .unwrap_or_else(|err| panic!("MCP GM: {err}"));
    let queue_args = json!({"actor_id": "aldric", "move": "wait", "id": "mcp-wait"});
    insert_json(&mut out, "mcp-call", tool_call(1, "queue", &queue_args));
    insert_json(&mut out, "mcp-result", call(&mut mcp, "queue", queue_args));
    send(&mut back, "grit", json!({}));
    next_kind(&back, "queued");
    insert_json(&mut out, "mcp-event", one_event(&mut mcp));
    let mut newcomer = RawSession::open(table.mcp_port);
    let refused = newcomer.open_event_stream();
    insert_json(&mut out, "mcp-table-full", read_event(&refused));

    send(&mut back, "help", json!({}));
    insert_json(&mut out, "player-help", next_kind(&back, "help"));
    send(&mut back, "map", json!({}));
    insert_json(&mut out, "player-map", next_kind(&back, "map"));
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

/// Opens a connection to `/player` or `/gm`. The hello is its first message.
fn open_seat(port: u16, seat: &str) -> SeatClient {
    let stream = connect(port);
    let write = stream
        .try_clone()
        .unwrap_or_else(|err| panic!("clone: {err}"));
    let url = format!("ws://127.0.0.1:{port}/{seat}");
    let (mut read, _) =
        tungstenite::client(url, stream).unwrap_or_else(|err| panic!("{seat} seat: {err}"));
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
    SeatClient {
        write: WebSocket::from_raw_socket(write, Role::Client, None),
        lines,
        next_id: 1,
    }
}

/// Sends `word` with `args` under the next id. Returns the message sent.
fn send(seat: &mut SeatClient, word: &str, args: Value) -> Value {
    let id = format!("c{}", seat.next_id);
    seat.next_id += 1;
    let message = serde_json::to_value(ClientMessage::new(&id, word, args))
        .unwrap_or_else(|err| panic!("{word}: {err}"));
    seat.write
        .send(Message::text(message.to_string()))
        .unwrap_or_else(|err| panic!("send: {err}"));
    message
}

fn next_json(seat: &SeatClient) -> Value {
    let line = seat
        .lines
        .recv_timeout(READ_TIMEOUT)
        .unwrap_or_else(|err| panic!("no message: {err}"));
    serde_json::from_str(&line).unwrap_or_else(|err| panic!("{err} in {line}"))
}

/// The next message, which must have a `kind` body.
fn next_kind(seat: &SeatClient, kind: &str) -> Value {
    let message = next_json(seat);
    assert_eq!(message["body"]["kind"], kind, "{message}");
    message
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
    let mut player = open_unclaimed(port);
    send(&mut player, word, args);
    next_json(&player)
}

/// A player connection past its hello, with no claim.
fn open_unclaimed(port: u16) -> SeatClient {
    let player = open_seat(port, "player");
    next_kind(&player, "hello");
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
