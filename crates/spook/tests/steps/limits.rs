use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cucumber::{then, when};

use spookery::host::{OUTBOUND_BYTES, OUTBOUND_FRAMES, REQUESTED_SEND_BUFFER};
use spookery::protocol::{write_player, PlayerMessage};

use crate::support;
use crate::World;

use super::table;

const MIB: usize = 1024 * 1024;
const SAY_TEXT_BYTES: usize = 4 * 1024;

#[when(expr = "a player sends a 1 MiB line with no newline")]
fn a_player_sends_a_mebibyte_line(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    table::seat_player(&pack);
    let text = crate::with_live(|live| Arc::clone(&live.player_text));
    let writer = crate::with_live(|live| {
        live.player_write
            .as_ref()
            .unwrap_or_else(|| panic!("no player"))
            .try_clone()
            .unwrap_or_else(|err| panic!("clone: {err}"))
    });
    let join = std::thread::spawn(move || write_all_ignoring(writer, vec![b'a'; MIB]));
    world.player_added = wait_text(&text, "\"code\":\"LINE_TOO_LONG\"", Duration::from_secs(8));
    table::wait_humans(&pack, 0);
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
    assert_eq!(table::live_presence().0, 0, "the seat was not released");
    table::release_status_gm();
}

#[then(expr = "a second player can sit and wait")]
fn a_second_player_can_sit_and_wait(world: &mut World) {
    table::release_status_gm();
    let pack = table::pack(world).to_path_buf();
    table::seat_actor(&pack, "pc_harbormouse");
    let _ = table::actor_send("pc_harbormouse", "wait", "\"kind\":");
    assert!(
        !table::list_tick_keys(&pack).is_empty(),
        "the second player did not commit a tick"
    );
}

#[when(expr = "a socket sends an oversized hello")]
fn a_socket_sends_an_oversized_hello(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    let stream = connect(crate::with_live(|live| live.port));
    let text = spawn_reader(&stream);
    let join = std::thread::spawn(move || write_all_ignoring(stream, vec![b'a'; MIB]));
    world.scratch = wait_text(&text, "\"code\":\"LINE_TOO_LONG\"", Duration::from_secs(8));
    join.join().unwrap_or_else(|_| panic!("writer thread"));
}

#[when(expr = "an HTTP body over 1 MiB is posted")]
fn an_http_body_over_a_mebibyte(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    let http = crate::with_live(|live| live.http_port);
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
fn a_new_gm_can_sit(_world: &mut World) {
    let port = crate::with_live(|live| live.http_port);
    let start = Instant::now();
    loop {
        match try_gm(port) {
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
    let mark = table::player_len();
    crate::with_live(|live| {
        let write = live
            .player_write
            .as_mut()
            .unwrap_or_else(|| panic!("no player"));
        write
            .write_all(b"\xff\n")
            .unwrap_or_else(|err| panic!("write: {err}"));
        write.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    });
    world.player_added = table::wait_player_from(mark, "\"code\":\"SCHEMA\"");
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
    let _ = table::player_send("wait", "\"kind\":");
    assert!(
        table::now_of(table::pack(world)).world_time > 1080,
        "the line after invalid UTF-8 did not commit"
    );
}

#[when(expr = "a socket connects and sends nothing")]
fn a_socket_connects_and_sends_nothing(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen_with(&pack, &[("SPOOK_HELLO_TIMEOUT_MS", "200")]);
    let mut stream = connect(crate::with_live(|live| live.port));
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
    let before = table::list_tick_keys(&pack).len();
    let started = Instant::now();
    crate::with_live(|live| {
        let player = live
            .players
            .iter_mut()
            .find(|player| player.actor_id == "pc_rowan")
            .unwrap_or_else(|| panic!("no rowan"));
        for _ in 0..200 {
            writeln!(player.write, "wait").unwrap_or_else(|err| panic!("write: {err}"));
        }
        player
            .write
            .flush()
            .unwrap_or_else(|err| panic!("flush: {err}"));
    });
    let other_at = Instant::now();
    let _ = table::actor_send("pc_harbormouse", "wait", "\"kind\":");
    let other_ms = other_at.elapsed().as_millis();
    // Past the burst means a later line was kept. A drop would stop at 11 ticks.
    let deadline = Instant::now() + Duration::from_secs(8);
    let ticks = loop {
        let ticks = table::list_tick_keys(&pack).len().saturating_sub(before);
        if ticks >= 12 || Instant::now() > deadline {
            break ticks;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    world.scratch = format!("{ticks} {} {other_ms}", started.elapsed().as_millis());
}

#[then(expr = "the committed ticks stay within the rate allowance")]
fn ticks_stay_within_the_allowance(world: &mut World) {
    let mut parts = world.scratch.split_whitespace();
    let ticks: f64 = parts
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
        ticks >= 12.0 && ticks <= allowance && ticks < 200.0,
        "ticks {ticks} allowance {allowance} ({})",
        world.scratch
    );
}

#[then(expr = "the other player's line commits within 2 seconds")]
fn the_other_player_commits_within_2_seconds(world: &mut World) {
    let other_ms: u128 = world
        .scratch
        .split_whitespace()
        .nth(2)
        .unwrap_or("99999")
        .parse()
        .unwrap_or(99999);
    assert!(other_ms < 2_000, "{}", world.scratch);
}

#[when(expr = "one player stops reading and the GM says many lines")]
fn one_player_stops_reading(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    table::seat_gm();
    table::seat_actor(&pack, "pc_harbormouse");
    let (mut quiet, recv) = connect_small_window(crate::with_live(|live| live.port));
    let send = granted_send_buffer(REQUESTED_SEND_BUFFER);
    writeln!(quiet, "player:pc_rowan").unwrap_or_else(|err| panic!("hello: {err}"));
    quiet.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    read_until(&mut quiet, "\"kind\":\"perceive\"", Duration::from_secs(2));
    crate::with_live(|live| {
        live.players.push(table::Seated {
            actor_id: "pc_rowan".to_string(),
            write: quiet,
            text: Arc::new(Mutex::new(String::new())),
        });
    });
    let say = "n".repeat(SAY_TEXT_BYTES);
    let frame = write_player(&PlayerMessage::Say {
        from: "gm".to_string(),
        address: "table".to_string(),
        text: say.clone(),
    })
    .len();
    // Granted kernel windows, then the userspace queue, then one extra
    // buffer so a getsockopt that under-reports still reaches the drop.
    let queue = OUTBOUND_BYTES.min(OUTBOUND_FRAMES.saturating_mul(frame));
    let margin = recv.max(send).max(frame);
    let budget = recv
        .saturating_add(send)
        .saturating_add(queue)
        .saturating_add(margin);
    let start = Instant::now();
    let mut sent = 0usize;
    let args = format!(r#"{{"address":"table","text":"{say}"}}"#);
    loop {
        let (_, present) = table::live_presence();
        if !present.iter().any(|actor| actor == "pc_rowan") {
            break;
        }
        if start.elapsed() > Duration::from_secs(60) || sent > budget {
            let stderr = crate::with_live(|live| {
                live.server
                    .as_ref()
                    .map(|server| support::text_of(&server.stderr))
                    .unwrap_or_default()
            });
            panic!(
                "rowan still seated after {sent} bytes (recv {recv}, send {send}, frame {frame}, queue {queue}, budget {budget}): {present:?}\n{stderr}"
            );
        }
        let _ = table::gm_call(2, "say", &args);
        sent = sent.saturating_add(frame);
    }
    let heard = Instant::now();
    loop {
        let text = table::actor_text("pc_harbormouse");
        if text.contains("\"kind\":\"say\"") {
            world.player_added = text;
            break;
        }
        if heard.elapsed() > Duration::from_secs(2) {
            panic!(
                "the other player saw no say\n{}",
                table::actor_text("pc_harbormouse")
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[then(expr = "that player is disconnected")]
fn that_player_is_disconnected(_world: &mut World) {
    let (_, present) = table::live_presence();
    assert!(
        !present.iter().any(|actor| actor == "pc_rowan"),
        "{present:?}"
    );
    assert!(
        present.iter().any(|actor| actor == "pc_harbormouse"),
        "{present:?}"
    );
    table::release_status_gm();
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

fn try_gm(port: u16) -> Result<(), String> {
    match spookery::gm_connection::GmConnection::connect_door(&format!("127.0.0.1:{port}")) {
        Ok(gm) => {
            drop(gm);
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    }
}
