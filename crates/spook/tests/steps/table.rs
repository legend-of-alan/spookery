use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use spookery::gm_connection::{GmConnection, Incoming};
use spookery_kernel::model::Now;

use crate::support::{self, Proc};
use crate::World;

pub struct Seated {
    pub actor_id: String,
    pub write: TcpStream,
    pub text: Arc<Mutex<String>>,
}

pub struct Live {
    pub port: u16,
    pub player_write: Option<TcpStream>,
    pub player_text: Arc<Mutex<String>>,
    pub players: Vec<Seated>,
    pub gm: Option<GmConnection>,
    pub http_port: u16,
    pub gm_text: Arc<Mutex<String>>,
    pub opened_now: Vec<u8>,
    pub seated_now: Vec<u8>,
    pub seated_rev: u64,
    pub server: Option<Proc>,
}

pub fn files(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes =
                    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

pub fn pack(world: &World) -> &Path {
    world
        .pack
        .as_deref()
        .unwrap_or_else(|| panic!("no docks table"))
}

/// Read `state/save.yaml` without opening the shard. `World::open` runs
/// recovery, which writes, and races a live server mid-commit. The server
/// replaces save.yaml by atomic rename, so a read is always whole. Windows
/// can briefly refuse to open a file mid-rename, so a failed read retries.
pub fn now_of(pack: &Path) -> Now {
    let path = pack.join("state/save.yaml");
    let start = Instant::now();
    let text = loop {
        match fs::read_to_string(&path) {
            Ok(text) => break text,
            Err(err) if start.elapsed() > Duration::from_secs(2) => {
                panic!("read save.yaml: {err}")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    spookery_kernel::store::codec::load_save(&text)
        .unwrap_or_else(|err| panic!("load now: {err:?}"))
}

pub fn tick_names(pack: &Path) -> Vec<String> {
    let dir = pack.join("state/journal");
    let mut names = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(&dir).unwrap_or_else(|err| panic!("read log: {err}")) {
            let entry = entry.unwrap_or_else(|err| panic!("dirent: {err}"));
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("tick-") && name.ends_with(".jsonl") {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

pub fn listen(pack: &Path, bind: &str, port: u16) {
    listen_with(pack, bind, port, &[]);
}

/// `spook run --mcp-http` on a port that is not the game port plus one.
pub fn listen_door(pack: &Path) -> u16 {
    let game = free_port();
    let http = loop {
        let port = free_port();
        if port != game && port != game.saturating_add(1) {
            break port;
        }
    };
    let pack_arg = support::pack_arg(pack);
    let addr = format!("127.0.0.1:{game}");
    let http_addr = format!("127.0.0.1:{http}");
    let mut server = support::spawn(&["run", &pack_arg, "--addr", &addr, "--mcp-http", &http_addr]);
    wait_listen(game, &mut server);
    wait_listen(http, &mut server);
    let opened_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    crate::set_live(Live {
        port: game,
        player_write: None,
        player_text: Arc::new(Mutex::new(String::new())),
        players: Vec::new(),
        gm: None,
        http_port: http,
        gm_text: Arc::new(Mutex::new(String::new())),
        opened_now: opened_now.clone(),
        seated_now: opened_now,
        seated_rev: 0,
        server: Some(server),
    });
    http
}

pub fn listen_with(pack: &Path, bind: &str, port: u16, env: &[(&str, &str)]) {
    let pack_arg = support::pack_arg(pack);
    let mut server = support::spawn_env(&["run", &pack_arg, "--addr", bind], env);
    wait_listen(port, &mut server);
    let opened_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    crate::set_live(Live {
        port,
        player_write: None,
        player_text: Arc::new(Mutex::new(String::new())),
        players: Vec::new(),
        gm: None,
        http_port: port.saturating_add(1),
        gm_text: Arc::new(Mutex::new(String::new())),
        opened_now: opened_now.clone(),
        seated_now: opened_now,
        seated_rev: 0,
        server: Some(server),
    });
}

pub fn seat_player(pack: &Path) {
    seat_actor(pack, "pc_rowan");
}

pub fn seat_actor(pack: &Path, actor_id: &str) {
    let port = crate::with_live(|live| live.port);
    let (write, text) = connect_actor(port, actor_id);
    let seated_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    let seated_rev = now_of(pack).state_rev;
    crate::with_live(|live| {
        if live.player_write.is_none() {
            live.player_write = Some(
                write
                    .try_clone()
                    .unwrap_or_else(|err| panic!("clone player: {err}")),
            );
            live.player_text = Arc::clone(&text);
        }
        live.players.push(Seated {
            actor_id: actor_id.to_string(),
            write,
            text,
        });
        live.seated_now = seated_now;
        live.seated_rev = seated_rev;
    });
}

pub fn seat_gm() {
    let http = crate::with_live(|live| live.http_port);
    let below = http
        .checked_sub(1)
        .unwrap_or_else(|| panic!("http port {http}"));
    let gm = GmConnection::connect(&format!("127.0.0.1:{below}"))
        .unwrap_or_else(|err| panic!("seat GM: {err}"));
    let opened = gm.opened_line().to_string();
    crate::with_live(|live| {
        push_line(&live.gm_text, &opened);
        live.gm = Some(gm);
    });
}

pub fn player_len() -> usize {
    crate::with_live(|live| support::text_of(&live.player_text).len())
}

pub fn player_send(line: &str, needle: &str) -> (usize, String) {
    let (slot, mark) = crate::with_live(|live| {
        let mark = support::text_of(&live.player_text).len();
        let write = live
            .player_write
            .as_mut()
            .unwrap_or_else(|| panic!("no seated player"));
        writeln!(write, "{line}").unwrap_or_else(|err| panic!("write: {err}"));
        write.flush().unwrap_or_else(|err| panic!("flush: {err}"));
        (Arc::clone(&live.player_text), mark)
    });
    (mark, wait_suffix(&slot, mark, needle))
}

pub fn wait_player_from(mark: usize, needle: &str) -> String {
    let slot = crate::with_live(|live| Arc::clone(&live.player_text));
    wait_suffix(&slot, mark, needle)
}

pub fn gm_call(id: u64, name: &str, arguments: &str) -> String {
    let value: Value =
        serde_json::from_str(arguments).unwrap_or_else(|err| panic!("arguments: {err}"));
    crate::with_live(|live| {
        let gm = live.gm.as_mut().unwrap_or_else(|| panic!("no seated GM"));
        let outcome = gm
            .call(name, value)
            .unwrap_or_else(|err| panic!("gm call: {err}"));
        for event in gm.take_events() {
            push_line(&live.gm_text, &event.raw);
        }
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "structuredContent": outcome.body }
        })
        .to_string();
        push_line(&live.gm_text, &raw);
        raw
    })
}

pub fn wait_gm(needle: &str) -> String {
    let start = Instant::now();
    loop {
        let text = gm_text();
        if text.contains(needle) {
            return text;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("missing {needle}\n{text}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_world_time(pack: &Path, expected: u64) {
    let start = Instant::now();
    let path = pack.join("state/save.yaml");
    loop {
        let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read save.yaml: {err}"));
        if let Ok(now) = spookery_kernel::store::codec::load_save(&text) {
            if now.world_time == expected {
                return;
            }
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("world_time did not become {expected}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn gm_text() -> String {
    crate::with_live(|live| {
        if let Some(gm) = live.gm.as_mut() {
            let ready = gm.drain_ready();
            for incoming in ready {
                if let Some(line) = incoming_line(incoming) {
                    push_line(&live.gm_text, &line);
                }
            }
        }
        support::text_of(&live.gm_text)
    })
}

fn push_line(slot: &Arc<Mutex<String>>, line: &str) {
    let mut text = slot.lock().unwrap_or_else(|err| err.into_inner());
    text.push_str(line);
    if !line.ends_with('\n') {
        text.push('\n');
    }
}

fn incoming_line(incoming: Incoming) -> Option<String> {
    match incoming {
        Incoming::Event(event) => Some(event.raw),
        Incoming::Result { raw, .. } | Incoming::Bad(raw) | Incoming::Refused { raw, .. } => {
            Some(raw)
        }
        Incoming::Eof => None,
    }
}

pub fn actor_text(actor_id: &str) -> String {
    let slot = crate::with_live(|live| {
        Arc::clone(
            &live
                .players
                .iter()
                .find(|player| player.actor_id == actor_id)
                .unwrap_or_else(|| panic!("no seat for {actor_id}"))
                .text,
        )
    });
    support::text_of(&slot)
}

pub fn actor_send(actor_id: &str, line: &str, needle: &str) -> String {
    let (slot, mark) = crate::with_live(|live| {
        let player = live
            .players
            .iter_mut()
            .find(|player| player.actor_id == actor_id)
            .unwrap_or_else(|| panic!("no seat for {actor_id}"));
        let mark = support::text_of(&player.text).len();
        writeln!(player.write, "{line}").unwrap_or_else(|err| panic!("write: {err}"));
        player
            .write
            .flush()
            .unwrap_or_else(|err| panic!("flush: {err}"));
        (Arc::clone(&player.text), mark)
    });
    wait_suffix(&slot, mark, needle)
}

pub fn stop_server() {
    crate::with_live(|live| {
        live.players.clear();
        live.player_write.take();
        live.gm.take();
        if let Some(server) = live.server.take() {
            drop(server);
        }
    });
}

pub fn close_actor(actor_id: &str) {
    crate::with_live(|live| {
        let player = live
            .players
            .iter_mut()
            .find(|player| player.actor_id == actor_id)
            .unwrap_or_else(|| panic!("no seat for {actor_id}"));
        writeln!(player.write, "close").unwrap_or_else(|err| panic!("close: {err}"));
        player
            .write
            .flush()
            .unwrap_or_else(|err| panic!("flush: {err}"));
    });
}

pub fn wait_humans(pack: &Path, expected: u32) {
    let start = Instant::now();
    let path = pack.join("state/save.yaml");
    loop {
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(now) = spookery_kernel::store::codec::load_save(&text) {
                if now.presence.humans == expected {
                    return;
                }
            }
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("humans did not become {expected}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn second_hello(line: &str) -> String {
    let port = crate::with_live(|live| live.port);
    let mut stream = connect(port);
    writeln!(stream, "{line}").unwrap_or_else(|err| panic!("write: {err}"));
    stream.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    read_line(&mut stream)
}

/// Ports already handed to a scenario in this test process, each with its agent door (port + 1).
/// Scenarios run concurrently, and the OS may hand a just-probed port + 1 to the next probe
/// (Windows allocates ephemeral ports in order), so a probe alone cannot keep two tables apart.
static RESERVED: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

pub fn free_port() -> u16 {
    loop {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
        let port = listener
            .local_addr()
            .unwrap_or_else(|err| panic!("local_addr: {err}"))
            .port();
        drop(listener);
        // The documented default is 127.0.0.1:9737. A free-port scenario asserts it was not chosen.
        // The agent door is the next port, so that port has to bind too.
        if port == 9737 || port.saturating_add(1) == 9737 {
            continue;
        }
        let agent = match std::net::TcpListener::bind(("127.0.0.1", port.saturating_add(1))) {
            Ok(agent) => agent,
            Err(_) => continue,
        };
        drop(agent);
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
            continue;
        }
        let door = port.saturating_add(1);
        let mut reserved = RESERVED
            .lock()
            .unwrap_or_else(|err| panic!("reserved: {err}"));
        if reserved.contains(&port) || reserved.contains(&door) {
            continue;
        }
        reserved.insert(port);
        reserved.insert(door);
        return port;
    }
}

fn wait_listen(port: u16, server: &mut Proc) {
    let start = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    loop {
        if TcpStream::connect(&addr).is_ok() {
            return;
        }
        if let Some(status) = server
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            panic!(
                "spook run exited {:?}\n{}",
                status.code(),
                support::text_of(&server.stderr)
            );
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!(
                "listener did not open\n{}",
                support::text_of(&server.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn connect(port: u16) -> TcpStream {
    let start = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    loop {
        match TcpStream::connect(&addr) {
            Ok(stream) => {
                stream
                    .set_nodelay(true)
                    .unwrap_or_else(|err| panic!("nodelay: {err}"));
                return stream;
            }
            Err(err) => {
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("connect {addr}: {err}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn connect_actor(port: u16, actor_id: &str) -> (TcpStream, Arc<Mutex<String>>) {
    let mut stream = connect(port);
    let text = spawn_reader(&stream);
    writeln!(stream, "player:{actor_id}").unwrap_or_else(|err| panic!("hello: {err}"));
    stream.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    wait_text(&text, "\"kind\":\"perceive\"");
    (stream, text)
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
                Ok(n) => slot
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });
    text
}

fn wait_text(slot: &Arc<Mutex<String>>, needle: &str) {
    let _ = wait_suffix(slot, 0, needle);
}

fn wait_suffix(slot: &Arc<Mutex<String>>, mark: usize, needle: &str) -> String {
    let start = Instant::now();
    loop {
        let text = support::text_of(slot);
        let added = text.get(mark..).unwrap_or("");
        if added.contains(needle) {
            return added.to_string();
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("missing {needle}\n{added}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn read_line(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
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

pub fn run(world: &mut World, args: &[&str]) {
    let output = support::spook(args);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}
