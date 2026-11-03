use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use cucumber::{then, when};
use serde_json::json;
use spookery_engine::testing::journal::list_round_keys;
use tungstenite::Message;

use spookery::protocol::{Say, ServerBody, ServerMessage, Speaker};

use crate::support;
use crate::World;

use super::table;

const MIB: usize = 1024 * 1024;
const SAY_TEXT_BYTES: usize = 4 * 1024;

// The limits `docs/seat-protocol.md` states.
/// A message past 8 KiB is `LINE_TOO_LONG`.
const MESSAGE_CAP: usize = 8 * 1024;
/// A connection that sends nothing for 10 seconds closes.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// A connection may burst to 10 messages. A seated player's claim spent one.
const BURST: usize = 10;
/// Then it may send 5 messages a second: one per 200 ms.
const REFILL: Duration = Duration::from_millis(200);
/// Each connection's outbound queue holds at most 256 messages.
const FRAME_CAP: usize = 256;
/// And at most 64 KiB.
const BYTE_CAP: usize = 64 * 1024;
/// The window this test asks for: the stopped reader's receive buffer, and
/// the send buffer it budgets for the host's side of that socket.
const WINDOW: usize = 8 * 1024;

#[when(expr = "the player sends a message over the cap")]
fn the_player_sends_a_message_over_the_cap(world: &mut World) {
    let player = table::player_client(world, "pc_rowan");
    let mark = player.seat.mark();
    let long = json!({ "id": "long", "body": { "word": "wait" }, "pad": "a".repeat(MESSAGE_CAP) });
    player.seat.send_raw(Message::text(long.to_string()));
    world.player_added = table::wait_player_from(world, mark, "\"code\":\"LINE_TOO_LONG\"");
}

#[when(expr = "a socket sends a 1 MiB message")]
fn a_socket_sends_a_mebibyte_message(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    let seat = table::Seat::open(table::live_mut(world).game_port, "player");
    let _ = seat.try_send_raw(Message::text("a".repeat(MIB)));
    seat.wait_closed();
    world.player_added = support::text_of(&seat.text);
}

#[then(expr = "the socket was closed without a reply")]
fn the_socket_was_closed_without_a_reply(world: &mut World) {
    assert!(
        !world.player_added.contains("\"reply_to\""),
        "{}",
        world.player_added
    );
}

#[then(expr = "a second player can sit and wait")]
fn a_second_player_can_sit_and_wait(world: &mut World) {
    table::release_probe_gm(world);
    let pack = table::pack(world).to_path_buf();
    let before = list_round_keys(&pack).len();
    table::seat_actor(world, &pack, "pc_harbormouse");
    table::actor_write(world, "pc_harbormouse", "wait");
    table::wait_committed_wait(&pack, "pc_harbormouse", before);
}

#[when(expr = "the GM releases the seat")]
fn the_gm_releases_the_seat(world: &mut World) {
    world.scratch = table::release_gm(world);
}

#[then(expr = "the server confirms the seat is free")]
fn the_server_confirms_the_seat_is_free(world: &mut World) {
    assert!(
        world.scratch.contains("\"released\":true"),
        "{}",
        world.scratch
    );
}

#[when(expr = "the GM releases the seat and keeps the session")]
fn the_gm_releases_the_seat_and_keeps_the_session(world: &mut World) {
    world.scratch = table::release_gm_keep_session(world);
}

#[when(expr = "another GM sits")]
fn another_gm_sits(world: &mut World) {
    table::seat_gm(world);
}

#[when(expr = "the closed session calls say")]
fn the_closed_session_calls_say(world: &mut World) {
    world.before = table::read_files(table::pack(world));
    world.scratch =
        table::closed_gm_call(world, "say", r#"{"address":"table","text":"good evening"}"#);
}

#[then(expr = "that call is refused and nothing was written")]
fn that_call_is_refused_and_nothing_was_written(world: &mut World) {
    assert!(
        world.scratch.contains("\"code\":\"NOT_IN_SPEC\""),
        "{}",
        world.scratch
    );
    assert_eq!(
        table::read_files(table::pack(world)),
        world.before,
        "the closed session wrote the pack"
    );
}

#[when(expr = "an HTTP body over 1 MiB is posted")]
fn an_http_body_over_a_mebibyte(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    let http = table::live_mut(world).mcp_port;
    let body = vec![b'x'; MIB + 1];
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{http}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut stream = connect(http);
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|err| panic!("write HTTP: {err}"));
    stream
        .write_all(&body)
        .unwrap_or_else(|err| panic!("write body: {err}"));
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    world.scratch = buf;
    world.pack = Some(pack);
}

#[then(expr = "a new GM can sit")]
fn a_new_gm_can_sit(world: &mut World) {
    let port = table::live_mut(world).mcp_port;
    let start = Instant::now();
    loop {
        match try_sit_gm(port) {
            Ok(()) => return,
            Err(err) if start.elapsed() < Duration::from_secs(5) => {
                let _ = err;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => panic!("new GM did not sit: {err}"),
        }
    }
}

#[when(expr = "the player sends a binary message")]
fn the_player_sends_a_binary_message(world: &mut World) {
    let player = table::player_client(world, "pc_rowan");
    let mark = player.seat.mark();
    player.seat.send_raw(Message::binary(vec![0xff]));
    world.player_added = table::wait_player_from(world, mark, "\"code\":\"SCHEMA\"");
}

#[when(expr = "the GM calls close with a key")]
fn the_gm_calls_close_with_a_key(world: &mut World) {
    world.scratch = table::gm_call(world, 91, "close", r#"{"now":true}"#);
}

#[then(expr = "that call is refused with {string}")]
fn that_call_is_refused_with(world: &mut World, code: String) {
    assert!(
        world.scratch.contains(&format!("\"code\":\"{code}\"")),
        "{}",
        world.scratch
    );
    assert!(
        !world.scratch.contains("\"released\":true"),
        "{}",
        world.scratch
    );
}

#[then(expr = "the GM can still say")]
fn the_gm_can_still_say(world: &mut World) {
    let text = table::gm_call(
        world,
        93,
        "say",
        r#"{"address":"table","text":"still here"}"#,
    );
    assert!(text.contains("\"seq\":"), "the GM lost the seat\n{text}");
}

#[then(expr = "the player is refused with {string}")]
fn the_player_is_refused_with(world: &mut World, code: String) {
    let needle = format!("\"code\":\"{code}\"");
    assert!(
        world.player_added.contains(&needle),
        "{}",
        world.player_added
    );
}

#[then(expr = "the player can wait")]
fn the_player_can_wait(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let before = list_round_keys(&pack).len();
    table::player_write(world, "wait");
    table::wait_committed_wait(&pack, "pc_rowan", before);
}

#[when(expr = "a socket connects and sends nothing")]
fn a_socket_connects_and_sends_nothing(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::serve_in_process(world, &pack);
    let seat = table::Seat::open(table::live_mut(world).game_port, "player");
    // The door sets its deadline before the table sends the hello.
    table::wait_suffix(&seat.text, 0, "\"kind\":\"hello\"");
    table::advance(world, HELLO_TIMEOUT);
    seat.wait_closed();
    world.hello_closed = true;
}

#[then(expr = "the socket is closed before the hello")]
fn the_socket_is_closed_before_the_hello(world: &mut World) {
    assert!(world.hello_closed);
}

#[when(expr = "one player sends 200 lines as fast as it can")]
fn one_player_sends_200_lines(world: &mut World) {
    // The flood's pace is paused time, so serve this table in process.
    let pack = table::pack(world).to_path_buf();
    table::serve_in_process(world, &pack);
    table::seat_actor(world, &pack, "pc_rowan");
    table::seat_actor(world, &pack, "pc_harbormouse");
    let before = list_round_keys(&pack).len();
    for (actor_id, count) in [("pc_rowan", 200), ("pc_harbormouse", 1)] {
        let player = table::player_client(world, actor_id);
        for _ in 0..count {
            player.seat.send("wait", json!({}));
        }
    }
    // The other player's wait is its own round. Its place among the new rounds
    // shows whether it waited behind the flood.
    let other_waits = |key: &String| {
        table::list_player_waits(&pack, key)
            .iter()
            .any(|id| id == "pc_harbormouse")
    };
    // Paused time refills nothing: the flood's burst and the other line commit.
    let keys = table::wait_rounds_until(&pack, "the burst and the other player's wait", |keys| {
        let added = keys.get(before..).unwrap_or_default();
        added.len() >= BURST && added.iter().any(other_waits)
    });
    let other_at = keys
        .get(before..)
        .unwrap_or_default()
        .iter()
        .position(other_waits)
        .unwrap_or_else(|| panic!("no other player round"))
        + 1;
    // A GM would queue the next line instead of committing it, so the status
    // probe leaves before time moves.
    let _ = table::read_presence(world);
    table::release_probe_gm(world);
    let paced = list_round_keys(&pack).len() - before;
    // One refill is one more line. A dropped line would stop the count here.
    table::advance(world, REFILL);
    let _ = table::wait_round_count(&pack, before + paced + 1);
    let _ = table::read_presence(world);
    table::release_probe_gm(world);
    world.flood_paced = paced;
    world.flood_refilled = list_round_keys(&pack).len() - before;
    world.flood_other_at = other_at;
}

#[then(expr = "the committed rounds stay within the rate allowance")]
fn rounds_stay_within_the_allowance(world: &mut World) {
    // What the claim left of the burst and the other player's line, then one line per
    // refill. The round after the refill is a line past the burst that was kept.
    assert_eq!(
        [world.flood_paced, world.flood_refilled],
        [BURST, BURST + 1]
    );
}

#[then(expr = "the other player's line commits before the flood's burst is spent")]
fn the_other_player_commits_inside_the_burst(world: &mut World) {
    // Each session sends one line at a time, so the other line waits behind at
    // most the flood's burst and commits before any refill.
    assert!(world.flood_other_at <= BURST, "{}", world.flood_other_at);
}

#[when(expr = "one player stops reading and the GM says many lines")]
fn one_player_stops_reading(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "pc_harbormouse");
    let port = table::live_mut(world).game_port;
    let (stream, recv) = connect_small_window(port);
    let send = granted_send_buffer(WINDOW);
    let quiet = table::Seat::from_stream(stream, port, "player", false);
    quiet.send("claim", json!({ "actor": "pc_rowan" }));
    table::wait_presence(world, "pc_rowan seated", |_, present| {
        present.iter().any(|actor| actor == "pc_rowan")
    });
    table::live_mut(world).players.push(table::PlayerClient {
        actor_id: "pc_rowan".to_string(),
        seat: quiet,
    });
    let say = "n".repeat(SAY_TEXT_BYTES);
    let frame = ServerMessage {
        id: None,
        seq: Some(u64::from(u32::MAX)),
        reply_to: None,
        body: ServerBody::Say(Say {
            from: Speaker::Gm,
            address: "table".to_string(),
            place: None,
            actor: None,
            recipients: Vec::new(),
            text: say.clone(),
        }),
    }
    .to_text()
    .len();
    // Granted kernel windows, then the userspace queue, then one extra
    // buffer so a getsockopt that under-reports still reaches the drop.
    let queue = BYTE_CAP.min(FRAME_CAP.saturating_mul(frame));
    let margin = recv.max(send).max(frame);
    let budget = recv
        .saturating_add(send)
        .saturating_add(queue)
        .saturating_add(margin);
    let start = Instant::now();
    let mut sent = 0usize;
    let args = format!(r#"{{"address":"table","text":"{say}"}}"#);
    loop {
        let (_, present) = table::read_presence(world);
        if !present.iter().any(|actor| actor == "pc_rowan") {
            break;
        }
        if start.elapsed() > Duration::from_secs(60) || sent > budget {
            let stderr = table::live_mut(world)
                .host
                .as_ref()
                .map(|server| support::text_of(&server.stderr))
                .unwrap_or_default();
            panic!(
                "rowan still seated after {sent} bytes (recv {recv}, send {send}, frame {frame}, queue {queue}, budget {budget}): {present:?}\n{stderr}"
            );
        }
        let _ = table::gm_call(world, 2, "say", &args);
        sent = sent.saturating_add(frame);
    }
    world.player_added = table::wait_actor_from(world, "pc_harbormouse", 0, "\"kind\":\"say\"");
}

#[then(expr = "that player is disconnected")]
fn that_player_is_disconnected(world: &mut World) {
    let (_, present) = table::read_presence(world);
    assert!(
        !present.iter().any(|actor| actor == "pc_rowan"),
        "{present:?}"
    );
    assert!(
        present.iter().any(|actor| actor == "pc_harbormouse"),
        "{present:?}"
    );
    table::release_probe_gm(world);
}

#[then(expr = "the other player keeps receiving frames")]
fn the_other_player_keeps_receiving_frames(world: &mut World) {
    assert!(
        world.player_added.contains("\"kind\":\"say\""),
        "{}",
        world.player_added
    );
}

fn connect_small_window(port: u16) -> (TcpStream, usize) {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap_or_else(|err| panic!("socket: {err}"));
    socket
        .set_recv_buffer_size(WINDOW)
        .unwrap_or_else(|err| panic!("recv buffer: {err}"));
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .unwrap_or_else(|err| panic!("addr: {err}"));
    socket
        .connect(&addr.into())
        .unwrap_or_else(|err| panic!("connect 127.0.0.1:{port}: {err}"));
    let size = socket
        .recv_buffer_size()
        .unwrap_or_else(|err| panic!("recv buffer size: {err}"));
    (socket.into(), size)
}

/// What this OS grants for the send buffer the game port requests.
/// Set after accept, which is when the listener sets it.
fn granted_send_buffer(requested: usize) -> usize {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("addr: {err}"));
    let client = std::thread::spawn(move || TcpStream::connect(addr));
    let (accepted, _) = listener
        .accept()
        .unwrap_or_else(|err| panic!("accept: {err}"));
    let socket = socket2::SockRef::from(&accepted);
    let _ = socket.set_send_buffer_size(requested);
    let size = socket
        .send_buffer_size()
        .unwrap_or_else(|err| panic!("send buffer size: {err}"));
    drop(accepted);
    drop(listener);
    let _ = client.join();
    size
}

fn connect(port: u16) -> TcpStream {
    let addr = format!("127.0.0.1:{port}");
    let stream = TcpStream::connect(&addr).unwrap_or_else(|err| panic!("connect {addr}: {err}"));
    stream
        .set_nodelay(true)
        .unwrap_or_else(|err| panic!("nodelay: {err}"));
    stream
}

fn try_sit_gm(port: u16) -> Result<(), String> {
    let mcp_addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    match support::block_on(spookery::mcp::GmClient::connect(mcp_addr)) {
        Ok(gm) => {
            support::block_on(gm.close());
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    }
}
