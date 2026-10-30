use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cucumber::{then, when};
use spookery_engine::testing::journal::list_round_keys;

use spookery::host::{OUTBOUND_BYTE_CAP, OUTBOUND_FRAME_CAP, REQUESTED_SEND_BUFFER};
use spookery::protocol::{emit_player, PlayerFrame};

use crate::support;
use crate::World;

use super::table;

const MIB: usize = 1024 * 1024;
const SAY_TEXT_BYTES: usize = 4 * 1024;

#[when(expr = "a player sends a 1 MiB line with no newline")]
fn a_player_sends_a_mebibyte_line(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    let text = Arc::clone(&table::live_mut(world).player_text);
    let writer = table::live_mut(world)
        .first_stream
        .as_ref()
        .unwrap_or_else(|| panic!("no player"))
        .try_clone()
        .unwrap_or_else(|err| panic!("clone: {err}"));
    let join = std::thread::spawn(move || write_all_ignoring(writer, vec![b'a'; MIB]));
    world.player_added = wait_text(&text, "\"code\":\"LINE_TOO_LONG\"", Duration::from_secs(8));
    table::wait_players(world, &pack, 0);
    join.join().unwrap_or_else(|_| panic!("writer thread"));
}

#[then(expr = "that player is refused with {string} and disconnected")]
fn refused_and_disconnected(world: &mut World, code: String) {
    let needle = format!("\"code\":\"{code}\"");
    assert!(
        world.player_added.contains(&needle),
        "{}",
        world.player_added
    );
    assert_eq!(
        table::read_presence(world).0,
        0,
        "the seat was not released"
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

#[when(expr = "a socket sends an oversized hello")]
fn a_socket_sends_an_oversized_hello(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    let stream = connect(table::live_mut(world).game_port);
    let text = spawn_reader(&stream);
    let join = std::thread::spawn(move || write_all_ignoring(stream, vec![b'a'; MIB]));
    world.scratch = wait_text(&text, "\"code\":\"LINE_TOO_LONG\"", Duration::from_secs(8));
    join.join().unwrap_or_else(|_| panic!("writer thread"));
}

#[when(expr = "an HTTP body over 1 MiB is posted")]
fn an_http_body_over_a_mebibyte(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    let http = table::live_mut(world).door_port;
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
    let port = table::live_mut(world).door_port;
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

#[when(expr = "the player sends invalid UTF-8")]
fn the_player_sends_invalid_utf8(world: &mut World) {
    let mark = table::player_mark(world);
    let live = table::live_mut(world);
    let write = live
        .first_stream
        .as_mut()
        .unwrap_or_else(|| panic!("no player"));
    write
        .write_all(b"\xff\n")
        .unwrap_or_else(|err| panic!("write: {err}"));
    write.flush().unwrap_or_else(|err| panic!("flush: {err}"));
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
    table::listen_with_env(world, &pack, &[("SPOOK_HELLO_TIMEOUT_MS", "200")]);
    let mut stream = connect(table::live_mut(world).game_port);
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    let start = Instant::now();
    let mut buf = [0u8; 8];
    let closed = match stream.read(&mut buf) {
        Ok(0) => true,
        Ok(_) => false,
        Err(err) if err.kind() == ErrorKind::TimedOut || err.kind() == ErrorKind::WouldBlock => {
            false
        }
        Err(_) => true,
    };
    world.scratch = format!("closed={closed} elapsed_ms={}", start.elapsed().as_millis());
}

#[then(expr = "the socket is closed before the hello")]
fn the_socket_is_closed_before_the_hello(world: &mut World) {
    assert!(
        world.scratch.starts_with("closed=true"),
        "{}",
        world.scratch
    );
    let ms: u128 = world
        .scratch
        .split("elapsed_ms=")
        .nth(1)
        .unwrap_or("99999")
        .parse()
        .unwrap_or(99999);
    assert!(ms < 2_000, "{}", world.scratch);
}

#[when(expr = "one player sends 200 lines as fast as it can")]
fn one_player_sends_200_lines(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let before = list_round_keys(&pack).len();
    let started = Instant::now();
    for (actor_id, count) in [("pc_rowan", 200), ("pc_harbormouse", 1)] {
        let player = table::live_mut(world)
            .players
            .iter_mut()
            .find(|player| player.actor_id == actor_id)
            .unwrap_or_else(|| panic!("no {actor_id}"));
        for _ in 0..count {
            writeln!(player.write, "wait").unwrap_or_else(|err| panic!("write: {err}"));
        }
        player
            .write
            .flush()
            .unwrap_or_else(|err| panic!("flush: {err}"));
    }
    // The other player's wait is its own round. Its place among the new rounds
    // shows whether it waited behind the flood.
    let other_waits = |key: &String| {
        table::list_player_waits(&pack, key)
            .iter()
            .any(|id| id == "pc_harbormouse")
    };
    let keys = table::wait_rounds_until(&pack, "the other player's wait", |keys| {
        keys.get(before..)
            .unwrap_or_default()
            .iter()
            .any(other_waits)
    });
    let other_at = keys
        .get(before..)
        .unwrap_or_default()
        .iter()
        .position(other_waits)
        .unwrap_or_else(|| panic!("no other player round"))
        + 1;
    // Past the burst means a later line was kept. A drop would stop at 11 rounds.
    let rounds = table::wait_round_count(&pack, before + 12).len() - before;
    world.scratch = format!("{rounds} {} {other_at}", started.elapsed().as_millis());
}

#[then(expr = "the committed rounds stay within the rate allowance")]
fn rounds_stay_within_the_allowance(world: &mut World) {
    let mut parts = world.scratch.split_whitespace();
    let rounds: f64 = parts
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or_else(|_| panic!("{}", world.scratch));
    let elapsed_ms: f64 = parts
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or_else(|_| panic!("{}", world.scratch));
    // Burst 10, then 5 lines per second, plus the other player's line and rounding.
    // 12 means a line past the burst was kept. 200 would mean the flood was not paced.
    let allowance = 10.0 + 5.0 * (elapsed_ms / 1000.0) + 2.0;
    assert!(
        rounds >= 12.0 && rounds <= allowance && rounds < 200.0,
        "rounds {rounds} allowance {allowance} ({})",
        world.scratch
    );
}

#[then(expr = "the other player's line commits before the flood's burst is spent")]
fn the_other_player_commits_inside_the_burst(world: &mut World) {
    let other_at: usize = world
        .scratch
        .split_whitespace()
        .nth(2)
        .unwrap_or("99999")
        .parse()
        .unwrap_or(99999);
    // Each session sends one line at a time, so the other line waits behind at
    // most the flood's burst of 10 and the line in flight, whatever the speed.
    assert!(other_at <= 12, "{}", world.scratch);
}

#[when(expr = "one player stops reading and the GM says many lines")]
fn one_player_stops_reading(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "pc_harbormouse");
    let (mut quiet, recv) = connect_small_window(table::live_mut(world).game_port);
    let send = granted_send_buffer(REQUESTED_SEND_BUFFER);
    writeln!(quiet, "player:pc_rowan").unwrap_or_else(|err| panic!("hello: {err}"));
    quiet.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    read_until(&mut quiet, "\"kind\":\"view\"", Duration::from_secs(2));
    table::live_mut(world).players.push(table::Seat {
        actor_id: "pc_rowan".to_string(),
        write: quiet,
        text: Arc::new(Mutex::new(String::new())),
    });
    let say = "n".repeat(SAY_TEXT_BYTES);
    let frame = emit_player(&PlayerFrame::Say {
        from: "gm".to_string(),
        address: "table".to_string(),
        text: say.clone(),
        replay: false,
    })
    .len();
    // Granted kernel windows, then the userspace queue, then one extra
    // buffer so a getsockopt that under-reports still reaches the drop.
    let queue = OUTBOUND_BYTE_CAP.min(OUTBOUND_FRAME_CAP.saturating_mul(frame));
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
    let heard = Instant::now();
    loop {
        let text = table::actor_text(world, "pc_harbormouse");
        if text.contains("\"kind\":\"say\"") {
            world.player_added = text;
            break;
        }
        if heard.elapsed() > Duration::from_secs(2) {
            panic!(
                "the other player saw no say\n{}",
                table::actor_text(world, "pc_harbormouse")
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
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

fn write_all_ignoring(mut stream: TcpStream, bytes: Vec<u8>) {
    let _ = stream.write_all(&bytes);
    let _ = stream.flush();
}

fn connect_small_window(port: u16) -> (TcpStream, usize) {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap_or_else(|err| panic!("socket: {err}"));
    socket
        .set_recv_buffer_size(REQUESTED_SEND_BUFFER)
        .unwrap_or_else(|err| panic!("recv buffer: {err}"));
    socket
        .set_tcp_nodelay(true)
        .unwrap_or_else(|err| panic!("nodelay: {err}"));
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .unwrap_or_else(|err| panic!("addr: {err}"));
    let start = Instant::now();
    loop {
        match socket.connect(&addr.into()) {
            Ok(()) => break,
            Err(err) => {
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("connect 127.0.0.1:{port}: {err}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let size = socket
        .recv_buffer_size()
        .unwrap_or_else(|err| panic!("recv buffer size: {err}"));
    (socket.into(), size)
}

/// What this OS grants for the send buffer the player socket requests.
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

fn spawn_reader(stream: &TcpStream) -> Arc<Mutex<String>> {
    let read = stream
        .try_clone()
        .unwrap_or_else(|err| panic!("clone: {err}"));
    let text = Arc::new(Mutex::new(String::new()));
    let slot = Arc::clone(&text);
    std::thread::spawn(move || {
        let mut read = read;
        let mut buf = [0u8; 256];
        loop {
            match read.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => text
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });
    slot
}

fn wait_text(slot: &Arc<Mutex<String>>, needle: &str, limit: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = support::text_of(slot);
        if text.contains(needle) {
            return text;
        }
        if start.elapsed() > limit {
            panic!("missing {needle}\n{text}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read_until(stream: &mut TcpStream, needle: &str, limit: Duration) {
    stream
        .set_read_timeout(Some(limit))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    let mut got = Vec::new();
    let mut tmp = [0u8; 1024];
    let start = Instant::now();
    while !String::from_utf8_lossy(&got).contains(needle) {
        if start.elapsed() > limit {
            panic!("missing {needle}\n{}", String::from_utf8_lossy(&got));
        }
        match stream.read(&mut tmp) {
            Ok(0) => panic!("eof before {needle}"),
            Ok(n) => got.extend_from_slice(&tmp[..n]),
            Err(err)
                if err.kind() == ErrorKind::TimedOut || err.kind() == ErrorKind::WouldBlock =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("read: {err}"),
        }
    }
    stream
        .set_read_timeout(None)
        .unwrap_or_else(|err| panic!("clear timeout: {err}"));
}

fn try_sit_gm(port: u16) -> Result<(), String> {
    let door = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    match support::block_on(spookery::gm_connection::Connection::connect_door(door)) {
        Ok(gm) => {
            support::block_on(gm.close());
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    }
}
