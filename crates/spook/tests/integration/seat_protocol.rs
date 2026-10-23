//! `docs/seat-protocol.md` is a capture of one live docks table.
//! Each fenced example is named. This test fails when a name is missing,
//! extra, or different from the table.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::ProtocolVersion;
use serde_json::Value;
use spookery::gm_connection::{GmConnection, Incoming};

/// A seat that never answers still has to fail the test. The line itself
/// unblocks the wait.
const HANG: Duration = Duration::from_secs(5);

const GAME_PORT: &str = "127.0.0.1:<game>";
const DOOR_PORT: &str = "127.0.0.1:<door>";

enum Example {
    Json(Value),
    Text(String),
}

struct Seat {
    write: TcpStream,
    lines: Receiver<String>,
}

struct Table {
    child: Child,
    _stdin: ChildStdin,
    pack: PathBuf,
    game: u16,
    door: u16,
    host_text: Arc<Mutex<String>>,
    host_more: Receiver<()>,
}

impl Drop for Table {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.pack);
    }
}

#[test]
fn the_seat_page_matches_a_live_table() {
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
    let mut player = seat(table.game, "pc_rowan");

    json(
        &mut out,
        "listening",
        serde_json::json!({
            "kind": "listening",
            "game": format!("127.0.0.1:{}", table.game),
            "door": format!("127.0.0.1:{}", table.door),
        }),
    );
    text(&mut out, "player-hello", "player:pc_rowan");
    let resume = next_json(&player);
    assert_eq!(resume["kind"], "resume", "{resume}");
    json(&mut out, "player-resume", resume);
    let perceive = next_json(&player);
    assert_eq!(perceive["kind"], "perceive", "{perceive}");
    json(&mut out, "player-perceive", perceive);

    text(&mut out, "player-look", "look");
    send(&mut player, "look");
    let lines = next_json(&player);
    assert_eq!(lines["kind"], "lines", "{lines}");
    json(&mut out, "player-lines", lines);

    json(
        &mut out,
        "player-seat-taken",
        refused(table.game, "player:pc_rowan"),
    );
    json(
        &mut out,
        "player-unknown-pc",
        refused(table.game, "player:no_such"),
    );
    json(
        &mut out,
        "player-gm-on-game-port",
        refused(table.game, "gm"),
    );
    json(&mut out, "player-line-too-long", too_long(table.game));

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
    json(&mut out, "initialize-request", initialize.clone());
    let (opened, session) = post_initialize(table.door, &initialize);
    json(&mut out, "initialize-result", opened.clone());
    let negotiated = opened["result"]["protocolVersion"]
        .as_str()
        .unwrap_or_else(|| panic!("no protocol version\n{opened}"))
        .to_string();
    // DELETE is a legacy-lifecycle call. The newest protocol version is refused
    // with 405, so the header is the version the handshake returned.
    delete_session(table.door, &session, &negotiated);

    let mut gm =
        GmConnection::connect_door(std::net::SocketAddr::from(([127, 0, 0, 1], table.door)))
            .unwrap_or_else(|err| panic!("seat GM: {err}"));

    let status_args = serde_json::json!({});
    json(
        &mut out,
        "session-status-request",
        tool_call(2, "session_status", &status_args),
    );
    json(
        &mut out,
        "session-status",
        call(&mut gm, "session_status", status_args),
    );

    let perceive_args = serde_json::json!({"as": "player:pc_rowan"});
    json(
        &mut out,
        "perceive-request",
        tool_call(3, "perceive", &perceive_args),
    );
    let gm_perceive = call(&mut gm, "perceive", perceive_args);
    assert_eq!(
        &gm_perceive,
        out_json(&out, "player-perceive"),
        "GM perceive diverged from the player welcome"
    );

    text(&mut out, "player-say", "say evening");
    send(&mut player, "say evening");
    let said = next_json(&player);
    assert_eq!(said["kind"], "say", "{said}");
    json(&mut out, "player-say-frame", said);
    json(&mut out, "say-event", one_event(&mut gm));

    let say_args = serde_json::json!({"address": "table", "text": "good evening"});
    json(&mut out, "say-request", tool_call(4, "say", &say_args));
    json(&mut out, "say-result", call(&mut gm, "say", say_args));
    let heard = next_json(&player);
    assert_eq!(heard["kind"], "say", "{heard}");
    json(&mut out, "player-hears-gm", heard);
    json(&mut out, "gm-say-event", one_event(&mut gm));

    let since_args = serde_json::json!({"since": 0});
    json(
        &mut out,
        "session-status-since-request",
        tool_call(5, "session_status", &since_args),
    );
    json(
        &mut out,
        "session-status-since",
        call(&mut gm, "session_status", since_args),
    );

    let intent_args = serde_json::json!({
        "actor_id": "pc_rowan",
        "actor_kind": "player",
        "source": "player",
        "move": "wait",
        "id": "gm-wait"
    });
    json(
        &mut out,
        "intent-request",
        tool_call(6, "intent", &intent_args),
    );
    json(
        &mut out,
        "intent-result",
        call(&mut gm, "intent", intent_args),
    );

    text(&mut out, "player-grit", "grit");
    send(&mut player, "grit");
    let queued = next_json(&player);
    assert_eq!(queued["kind"], "queued", "{queued}");
    json(&mut out, "player-queued", queued);
    json(&mut out, "queued-event", one_event(&mut gm));

    let refuse_commit = serde_json::json!({"include_tick": false});
    json(
        &mut out,
        "commit-without-tick-request",
        tool_call(7, "commit_round", &refuse_commit),
    );
    json(
        &mut out,
        "commit-without-tick",
        call(&mut gm, "commit_round", refuse_commit),
    );

    let commit_args = serde_json::json!({"include_tick": true});
    json(
        &mut out,
        "commit-request",
        tool_call(8, "commit_round", &commit_args),
    );
    json(
        &mut out,
        "commit-result",
        call(&mut gm, "commit_round", commit_args),
    );
    json(&mut out, "check-event", one_event(&mut gm));
    let check = next_json(&player);
    assert_eq!(check["kind"], "check", "{check}");
    json(&mut out, "player-check", check);
    let again = next_json(&player);
    assert_eq!(again["kind"], "resume", "{again}");
    let refreshed = next_json(&player);
    assert_eq!(refreshed["kind"], "perceive", "{refreshed}");

    // A second round moves an item the player can see, so the change line is not empty.
    send(&mut player, "take lamp_oil");
    let queued_take = next_json(&player);
    assert_eq!(queued_take["kind"], "queued", "{queued_take}");
    let _ = one_event(&mut gm);
    call(
        &mut gm,
        "commit_round",
        serde_json::json!({"include_tick": true}),
    );
    let taken = next_json(&player);
    assert_eq!(taken["kind"], "resume", "{taken}");
    let after_take = next_json(&player);
    assert_eq!(after_take["kind"], "perceive", "{after_take}");
    let changes = next_json(&player);
    assert_eq!(changes["kind"], "lines", "{changes}");
    json(&mut out, "player-changes", changes);

    text(&mut out, "player-unknown-line", "blah");
    send(&mut player, "blah");
    json(&mut out, "player-not-in-spec", next_json(&player));

    text(&mut out, "player-empty-say", "say");
    send(&mut player, "say");
    json(&mut out, "player-schema", next_json(&player));

    text(&mut out, "player-locked-exit", "travel stairs");
    send(&mut player, "travel stairs");
    json(&mut out, "player-no-exit", next_json(&player));

    let bad_tool = serde_json::json!({});
    json(
        &mut out,
        "unknown-tool-request",
        tool_call(9, "not_in_spec", &bad_tool),
    );
    json(
        &mut out,
        "unknown-tool",
        call(&mut gm, "not_in_spec", bad_tool),
    );

    let unscoped = serde_json::json!({});
    json(
        &mut out,
        "unscoped-perceive-request",
        tool_call(10, "perceive", &unscoped),
    );
    json(
        &mut out,
        "unscoped-perceive",
        call(&mut gm, "perceive", unscoped),
    );

    let mouse = seat(table.game, "pc_harbormouse");
    let wren = seat(table.game, "pc_wren");
    // The welcome line means that hello has the seat. The fourth hello comes after both.
    assert_eq!(next_json(&mouse)["kind"], "resume");
    assert_eq!(next_json(&wren)["kind"], "resume");
    json(
        &mut out,
        "player-session-full",
        refused(table.game, "player:pc_extra"),
    );

    json(
        &mut out,
        "gm-session-full",
        post_full(table.door, &initialize),
    );

    let mark = table.host_text.lock().unwrap().len();
    player
        .write
        .shutdown(std::net::Shutdown::Both)
        .unwrap_or_else(|err| panic!("shutdown: {err}"));
    drop(player);
    wait_host_grew(&table, mark);
    let again = seat(table.game, "pc_rowan");
    let replay = next_json(&again);
    assert_eq!(replay["kind"], "say", "{replay}");
    assert_eq!(replay["replay"], true, "{replay}");
    json(&mut out, "player-replay-say", replay);

    let close_args = serde_json::json!({});
    json(
        &mut out,
        "close-request",
        tool_call(11, "close", &close_args),
    );
    json(&mut out, "close-result", call(&mut gm, "close", close_args));
    out
}

fn listen() -> Table {
    let pack = crate::support::temp_pack("seat-protocol");
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
    let (game, door) = read_listening(&mut stdout);
    let host_text = Arc::new(Mutex::new(String::new()));
    let (host_tx, host_more) = mpsc::channel();
    let slot = Arc::clone(&host_text);
    std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut buf = [0u8; 256];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    slot.lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(&buf[..n]));
                    let _ = host_tx.send(());
                }
            }
        }
    });
    Table {
        child,
        _stdin: stdin,
        pack,
        game: game.port(),
        door: door.port(),
        host_text,
        host_more,
    }
}

fn wait_host_grew(table: &Table, mark: usize) {
    // Assumes the next host output after `mark` is the seat-release summary
    // (not a late-buffered sit line). An empty summary would hang until HANG.
    let start = std::time::Instant::now();
    loop {
        if table.host_text.lock().unwrap().len() > mark {
            return;
        }
        let left = HANG.saturating_sub(start.elapsed());
        if left.is_zero() || table.host_more.recv_timeout(left).is_err() {
            panic!(
                "host did not print after the seat closed\n{}",
                table.host_text.lock().unwrap()
            );
        }
    }
}

fn read_listening(stdout: &mut impl Read) -> (std::net::SocketAddr, std::net::SocketAddr) {
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match stdout.read(&mut buf) {
            Ok(0) => panic!("spook run closed stdout\n{}", String::from_utf8_lossy(&got)),
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if let Some(end) = got.iter().position(|byte| *byte == b'\n') {
                    let line =
                        std::str::from_utf8(&got[..=end]).unwrap_or_else(|err| panic!("{err}"));
                    let value: Value = serde_json::from_str(line.trim())
                        .unwrap_or_else(|err| panic!("listening: {err}\n{line}"));
                    assert_eq!(value["kind"], "listening", "{value}");
                    let game = value["game"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{value}"))
                        .parse()
                        .unwrap_or_else(|err| panic!("{err}"));
                    let door = value["door"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{value}"))
                        .parse()
                        .unwrap_or_else(|err| panic!("{err}"));
                    return (game, door);
                }
            }
            Err(err) => panic!("stdout: {err}"),
        }
    }
}

fn seat(port: u16, actor: &str) -> Seat {
    let write = connect(port);
    let read = write
        .try_clone()
        .unwrap_or_else(|err| panic!("clone: {err}"));
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(read);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.ends_with('\n') {
                        line.pop();
                    }
                    if line.ends_with('\r') {
                        line.pop();
                    }
                    if tx.send(std::mem::take(&mut line)).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut seated = Seat { write, lines };
    send(&mut seated, &format!("player:{actor}"));
    seated
}

fn send(seat: &mut Seat, line: &str) {
    writeln!(seat.write, "{line}").unwrap_or_else(|err| panic!("write: {err}"));
    seat.write
        .flush()
        .unwrap_or_else(|err| panic!("flush: {err}"));
}

fn next_line(seat: &Seat) -> String {
    seat.lines
        .recv_timeout(HANG)
        .unwrap_or_else(|err| panic!("no player line: {err}"))
}

fn next_json(seat: &Seat) -> Value {
    let line = next_line(seat);
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

fn refused(port: u16, line: &str) -> Value {
    let mut stream = connect(port);
    stream
        .set_read_timeout(Some(HANG))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    writeln!(stream, "{line}").unwrap_or_else(|err| panic!("write: {err}"));
    stream.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    let text = read_line(&mut stream);
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{err} in {text}"))
}

fn too_long(port: u16) -> Value {
    let mut stream = connect(port);
    stream
        .set_read_timeout(Some(HANG))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    let mut bytes = vec![b'x'; 8 * 1024 + 1];
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .unwrap_or_else(|err| panic!("write: {err}"));
    stream.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    let text = read_line(&mut stream);
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{err} in {text}"))
}

fn read_line(stream: &mut TcpStream) -> String {
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
    String::from_utf8(buf).unwrap_or_else(|err| panic!("utf-8: {err}"))
}

fn call(gm: &mut GmConnection, name: &str, arguments: Value) -> Value {
    gm.call(name, arguments)
        .unwrap_or_else(|err| panic!("{name}: {err}"))
        .body
}

fn one_event(gm: &mut GmConnection) -> Value {
    let mut ready = gm.take_events();
    if ready.is_empty() {
        match gm.next_incoming() {
            Incoming::Event(event) => ready.push(event),
            other => panic!("expected a GM event, got {other:?}"),
        }
    }
    assert_eq!(ready.len(), 1, "GM events: {ready:?}");
    serde_json::from_str(&ready[0].raw).unwrap_or_else(|err| panic!("{err} in {}", ready[0].raw))
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
    let raw = http(
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

fn post_full(port: u16, body: &Value) -> Value {
    let raw = http(
        port,
        "POST",
        &serde_json::to_string(body).unwrap(),
        None,
        ProtocolVersion::LATEST.as_str(),
    );
    json_rpc(&raw).unwrap_or_else(|| panic!("no refusal\n{raw}"))
}

fn delete_session(port: u16, session: &str, version: &str) {
    let raw = http(port, "DELETE", "", Some(session), version);
    let status = raw.lines().next().unwrap_or("");
    assert!(
        status.contains(" 200 ") || status.contains(" 202 ") || status.contains(" 204 "),
        "DELETE did not release the seat\n{raw}"
    );
}

fn http(port: u16, method: &str, body: &str, session: Option<&str>, version: &str) -> String {
    let session = session
        .map(|id| format!("mcp-session-id: {id}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{session}MCP-Protocol-Version: {version}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = connect(port);
    stream
        .set_read_timeout(Some(HANG))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|err| panic!("write HTTP: {err}"));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        if http_complete(&buf) {
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

fn http_complete(buf: &[u8]) -> bool {
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

fn json(out: &mut BTreeMap<String, Example>, name: &str, value: Value) {
    out.insert(name.to_string(), Example::Json(value));
}

fn text(out: &mut BTreeMap<String, Example>, name: &str, line: &str) {
    out.insert(name.to_string(), Example::Text(line.to_string()));
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
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.get("kind").and_then(Value::as_str) != Some("listening") {
        return;
    }
    let game = object.get("game").and_then(Value::as_str).unwrap_or("");
    let door = object.get("door").and_then(Value::as_str).unwrap_or("");
    assert!(loopback_port(game).is_some(), "{game}");
    assert!(loopback_port(door).is_some(), "{door}");
    assert_ne!(game, door, "game and door ports");
    object.insert("game".to_string(), Value::String(GAME_PORT.to_string()));
    object.insert("door".to_string(), Value::String(DOOR_PORT.to_string()));
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
