use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use cucumber::{then, when};
use serde_json::json;
use spookery_engine::testing::journal::list_round_keys;
use tungstenite::Message;

use spookery::protocol::{Audience, ServerBody, ServerMessage, Speaker, Speech};

use crate::support;
use crate::World;

use super::table;

const MIB: usize = 1024 * 1024;
const SAY_TEXT_BYTES: usize = 4 * 1024;

/// A message past 8 KiB is `LINE_TOO_LONG`.
const MESSAGE_CAP: usize = 8 * 1024;
/// A connection that sends nothing for 10 seconds closes.
const FIRST_MESSAGE_TIMEOUT: Duration = Duration::from_secs(10);
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
async fn the_player_sends_a_message_over_the_cap(world: &mut World) {
    let player = table::player_client(world, "rowan");
    let mark = player.seat.mark();
    let long = json!({ "id": "long", "body": { "kind": "wait" }, "pad": "a".repeat(MESSAGE_CAP) });
    player.seat.send_raw(Message::text(long.to_string()));
    world.player_added = table::wait_player_from(world, mark, "\"code\":\"LINE_TOO_LONG\"").await;
}

#[when(expr = "a socket sends a 1 MiB message")]
async fn a_socket_sends_a_mebibyte_message(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack).await;
    let seat = table::Seat::open(table::live_mut(world).game_port, "player").await;
    let _ = seat.try_send_raw(Message::text("a".repeat(MIB)));
    seat.wait_closed().await;
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

#[then(expr = "a second player can claim and wait")]
async fn a_second_player_can_claim_and_wait(world: &mut World) {
    table::release_probe_gm(world).await;
    let pack = table::pack(world).to_path_buf();
    table::seat_actor(world, &pack, "harbormouse").await;
    table::actor_send(world, "harbormouse", "wait", r#""kind":"round""#).await;
}

#[when(expr = "the GM releases the seat")]
async fn the_gm_releases_the_seat(world: &mut World) {
    world.scratch = table::release_gm(world).await;
}

#[then(expr = "the server confirms the seat is free")]
fn the_server_confirms_the_seat_is_free(world: &mut World) {
    assert!(
        world.scratch.contains("{\"kind\":\"released\"}"),
        "{}",
        world.scratch
    );
}

#[when(expr = "the GM releases the seat and keeps the session")]
async fn the_gm_releases_the_seat_and_keeps_the_session(world: &mut World) {
    world.scratch = table::release_gm_keep_session(world).await;
}

#[when(expr = "another GM connects")]
async fn another_gm_connects(world: &mut World) {
    table::seat_gm(world).await;
}

#[when(expr = "the closed session calls say")]
async fn the_closed_session_calls_say(world: &mut World) {
    world.before = table::read_files(table::pack(world));
    world.scratch =
        table::closed_gm_tool_call(world, "say", r#"{"to":"table","text":"good evening"}"#).await;
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
async fn an_http_body_over_a_mebibyte(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack).await;
    let http = table::live_mut(world).mcp_port;
    let body = vec![b'x'; MIB + 1];
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{http}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut stream = connect(http);
    stream
        .set_read_timeout(Some(support::WAIT_LIMIT))
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

#[then(expr = "a new GM can connect")]
async fn a_new_gm_can_connect(world: &mut World) {
    let port = table::live_mut(world).mcp_port;
    table::poll_until("a new GM to connect", || try_connect_gm(port)).await;
}

#[when(expr = "the player sends a binary message")]
async fn the_player_sends_a_binary_message(world: &mut World) {
    let player = table::player_client(world, "rowan");
    let mark = player.seat.mark();
    player.seat.send_raw(Message::binary(vec![0xff]));
    world.player_added = table::wait_player_from(world, mark, "\"code\":\"SCHEMA\"").await;
}

#[when(expr = "the GM calls release with a key")]
async fn the_gm_calls_release_with_a_key(world: &mut World) {
    world.scratch = table::gm_tool_call(world, 91, "release", r#"{"now":true}"#).await;
}

#[then(expr = "that call is refused with {string}")]
fn that_call_is_refused_with(world: &mut World, code: String) {
    assert!(
        world.scratch.contains(&format!("\"code\":\"{code}\"")),
        "{}",
        world.scratch
    );
    assert!(
        !world.scratch.contains("{\"kind\":\"released\"}"),
        "{}",
        world.scratch
    );
}

#[then(expr = "the GM can still say")]
async fn the_gm_can_still_say(world: &mut World) {
    let text = table::gm_tool_call(world, 93, "say", r#"{"to":"table","text":"still here"}"#).await;
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
async fn the_player_can_wait(world: &mut World) {
    table::player_send(world, "wait", r#""kind":"round""#).await;
}

#[when(expr = "a socket connects and sends nothing")]
async fn a_socket_connects_and_sends_nothing(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::serve_in_process(world, &pack).await;
    let seat = table::Seat::open(table::live_mut(world).game_port, "player").await;
    table::wait_suffix(&seat.text, 0, "\"kind\":\"hello\"").await;
    table::advance(world, FIRST_MESSAGE_TIMEOUT).await;
    seat.wait_closed().await;
    world.silent_closed = true;
}

#[then(expr = "the socket is closed before the hello")]
fn the_socket_is_closed_before_the_hello(world: &mut World) {
    assert!(world.silent_closed);
}

#[when(expr = "one player sends 200 lines as fast as it can")]
async fn one_player_sends_200_lines(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::serve_in_process(world, &pack).await;
    table::seat_actor(world, &pack, "rowan").await;
    table::seat_actor(world, &pack, "harbormouse").await;
    let before = list_round_keys(&pack).len();
    for (actor_id, count) in [("rowan", 200), ("harbormouse", 1)] {
        let player = table::player_client(world, actor_id);
        for _ in 0..count {
            player.seat.send("wait", json!({}));
        }
    }
    let other_waits = |key: &String| {
        table::list_player_waits(&pack, key)
            .iter()
            .any(|id| id == "harbormouse")
    };
    let keys = table::wait_rounds_until(&pack, "the burst and the other player's wait", |keys| {
        let added = keys.get(before..).unwrap_or_default();
        added.len() >= BURST && added.iter().any(other_waits)
    })
    .await;
    let other_at = keys
        .get(before..)
        .unwrap_or_default()
        .iter()
        .position(other_waits)
        .unwrap_or_else(|| panic!("no other player round"))
        + 1;
    let _ = table::read_presence(world).await;
    table::release_probe_gm(world).await;
    let paced = list_round_keys(&pack).len() - before;
    table::advance(world, REFILL).await;
    let _ = table::wait_round_count(&pack, before + paced + 1).await;
    let _ = table::read_presence(world).await;
    table::release_probe_gm(world).await;
    world.flood_paced = paced;
    world.flood_refilled = list_round_keys(&pack).len() - before;
    world.flood_other_at = other_at;
}

#[then(expr = "the committed rounds stay within the rate allowance")]
fn rounds_stay_within_the_allowance(world: &mut World) {
    assert_eq!(
        [world.flood_paced, world.flood_refilled],
        [BURST, BURST + 1]
    );
}

#[then(expr = "the other player's line commits before the flood's burst is spent")]
fn the_other_player_commits_inside_the_burst(world: &mut World) {
    assert!(world.flood_other_at <= BURST, "{}", world.flood_other_at);
}

#[when(expr = "one player stops reading and the GM says many lines")]
async fn one_player_stops_reading(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack).await;
    table::seat_gm(world).await;
    table::seat_actor(world, &pack, "harbormouse").await;
    let port = table::live_mut(world).game_port;
    let (stream, recv) = connect_small_window(port);
    let send = granted_send_buffer(WINDOW);
    let quiet = table::Seat::from_stream(stream, port, "player", false);
    quiet.send("claim", json!({ "actor": "rowan" }));
    table::wait_presence(world, "rowan seated", |_, present| {
        present.iter().any(|actor| actor == "rowan")
    })
    .await;
    table::live_mut(world).players.push(table::PlayerClient {
        actor_id: "rowan".to_string(),
        seat: quiet,
    });
    let say = "n".repeat(SAY_TEXT_BYTES);
    let frame = ServerMessage {
        id: None,
        seq: Some(u64::from(u32::MAX)),
        reply_to: None,
        body: ServerBody::Say(Speech {
            from: Speaker::Gm,
            from_name: String::new(),
            to: Audience::Table,
            recipient_ids: Vec::new(),
            text: say.clone(),
        }),
    }
    .to_text()
    .len();
    let queue = BYTE_CAP.min(FRAME_CAP.saturating_mul(frame));
    let margin = recv.max(send).max(frame);
    let budget = recv
        .saturating_add(send)
        .saturating_add(queue)
        .saturating_add(margin);
    let start = Instant::now();
    let mut sent = 0usize;
    let args = format!(r#"{{"to":"table","text":"{say}"}}"#);
    while is_open(table::player_client(world, "rowan")) {
        if start.elapsed() > support::WAIT_LIMIT || sent > budget {
            let stderr = table::live_mut(world)
                .host
                .as_ref()
                .map(|server| support::text_of(&server.stderr))
                .unwrap_or_default();
            panic!(
                "rowan still connected after {sent} bytes (recv {recv}, send {send}, frame {frame}, queue {queue}, budget {budget})\n{stderr}"
            );
        }
        let _ = table::gm_tool_call(world, 2, "say", &args).await;
        sent = sent.saturating_add(frame);
    }
    world.player_added = table::wait_actor_from(world, "harbormouse", 0, "\"kind\":\"say\"").await;
}

/// A pong the table ignores, so writing it only tests the connection.
fn is_open(player: &table::PlayerClient) -> bool {
    player
        .seat
        .try_send_raw(Message::Pong(Default::default()))
        .is_ok()
}

#[then(expr = "that player is disconnected and still holds its PC")]
async fn that_player_is_disconnected(world: &mut World) {
    assert!(!is_open(table::player_client(world, "rowan")));
    let (_, present) = table::read_presence(world).await;
    assert!(present.iter().any(|actor| actor == "rowan"), "{present:?}");
    assert!(
        present.iter().any(|actor| actor == "harbormouse"),
        "{present:?}"
    );
    table::release_probe_gm(world).await;
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

async fn try_connect_gm(port: u16) -> Result<(), String> {
    let mcp_addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    match crate::mcp_client::GmClient::connect(mcp_addr).await {
        Ok(gm) => {
            gm.close().await;
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    }
}
