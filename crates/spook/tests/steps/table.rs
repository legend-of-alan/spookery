use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
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

pub struct Pending {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr: Arc<Mutex<String>>,
    stderr_drain: Option<JoinHandle<()>>,
}

pub struct Listening {
    pub game: SocketAddr,
    pub door: SocketAddr,
    pub server: Proc,
}

pub fn listen(pack: &Path) {
    let _ = listen_flags(pack, &["--addr", "127.0.0.1:0"], &[]);
}

/// `spook run` with an agent door the operating system chooses on its own.
pub fn listen_door(pack: &Path) -> u16 {
    listen_flags(
        pack,
        &["--addr", "127.0.0.1:0", "--mcp-http", "127.0.0.1:0"],
        &[],
    )
    .1
}

pub fn listen_with(pack: &Path, env: &[(&str, &str)]) {
    let _ = listen_flags(pack, &["--addr", "127.0.0.1:0"], env);
}

pub fn listen_flags(pack: &Path, flags: &[&str], env: &[(&str, &str)]) -> (u16, u16) {
    let listening = take_report(spawn_table(pack, flags, env));
    let ports = (listening.game.port(), listening.door.port());
    install(pack, listening);
    ports
}

pub fn game_addr() -> String {
    crate::with_live(|live| format!("127.0.0.1:{}", live.port))
}

pub fn door_addr() -> String {
    crate::with_live(|live| format!("127.0.0.1:{}", live.http_port))
}

/// The listening line `spook run` wrote, as the addresses it bound.
pub fn reported(stdout: &str) -> (SocketAddr, SocketAddr) {
    stdout
        .lines()
        .find_map(parse_line)
        .unwrap_or_else(|| panic!("no listening line\n{stdout}"))
}

pub fn spawn_table(pack: &Path, flags: &[&str], env: &[(&str, &str)]) -> Pending {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.arg("run").arg(support::pack_arg(pack)).args(flags);
    for (key, value) in env {
        command.env(key, value);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"));
    let stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .unwrap_or_else(|| panic!("stdout was not piped"));
    let stderr_pipe = child
        .stderr
        .take()
        .unwrap_or_else(|| panic!("stderr was not piped"));
    let stderr = Arc::new(Mutex::new(String::new()));
    let stderr_drain = drain(stderr_pipe, Arc::clone(&stderr));
    Pending {
        child,
        stdin,
        stdout,
        stderr,
        stderr_drain: Some(stderr_drain),
    }
}

pub fn take_report(mut pending: Pending) -> Listening {
    let (game, door, seen) = read_report(&mut pending);
    let stdout = Arc::new(Mutex::new(seen));
    drop(drain(pending.stdout, Arc::clone(&stdout)));
    Listening {
        game,
        door,
        server: Proc {
            child: pending.child,
            stdin: pending.stdin,
            stdout,
            stderr: pending.stderr,
        },
    }
}

fn install(pack: &Path, listening: Listening) {
    let opened_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    crate::set_live(Live {
        port: listening.game.port(),
        player_write: None,
        player_text: Arc::new(Mutex::new(String::new())),
        players: Vec::new(),
        gm: None,
        http_port: listening.door.port(),
        gm_text: Arc::new(Mutex::new(String::new())),
        opened_now: opened_now.clone(),
        seated_now: opened_now,
        seated_rev: 0,
        server: Some(listening.server),
    });
}

fn read_report(pending: &mut Pending) -> (SocketAddr, SocketAddr, String) {
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match pending.stdout.read(&mut buf) {
            Ok(0) => {
                let status = pending
                    .child
                    .wait()
                    .unwrap_or_else(|err| panic!("wait: {err}"));
                if let Some(drain) = pending.stderr_drain.take() {
                    let _ = drain.join();
                }
                panic!(
                    "spook run exited {status} before it reported its ports\nstdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&got),
                    support::text_of(&pending.stderr)
                );
            }
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if let Some((game, door)) = split_report(&got) {
                    let seen = String::from_utf8_lossy(&got).into_owned();
                    return (game, door, seen);
                }
            }
            Err(err) => panic!("read stdout: {err}"),
        }
    }
}

fn split_report(buf: &[u8]) -> Option<(SocketAddr, SocketAddr)> {
    let end = buf.iter().rposition(|byte| *byte == b'\n')? + 1;
    let text = std::str::from_utf8(&buf[..end]).unwrap_or_else(|err| panic!("stdout: {err}"));
    text.lines().find_map(parse_line)
}

fn parse_line(line: &str) -> Option<(SocketAddr, SocketAddr)> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("kind")?.as_str() != Some("listening") {
        return None;
    }
    let game = value.get("game")?.as_str()?.parse().ok()?;
    let door = value.get("door")?.as_str()?.parse().ok()?;
    Some((game, door))
}

fn drain(reader: impl Read + Send + 'static, slot: Arc<Mutex<String>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => slot
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    })
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
    let gm =
        GmConnection::connect_door(&door_addr()).unwrap_or_else(|err| panic!("seat GM: {err}"));
    let opened = gm.opened_line().to_string();
    crate::with_live(|live| {
        push_line(&live.gm_text, &opened);
        live.gm = Some(gm);
    });
}

pub fn player_len() -> usize {
    crate::with_live(|live| support::text_of(&live.player_text).len())
}

pub fn player_text() -> String {
    crate::with_live(|live| support::text_of(&live.player_text))
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

fn connect(port: u16) -> TcpStream {
    let addr = format!("127.0.0.1:{port}");
    let stream = TcpStream::connect(&addr).unwrap_or_else(|err| panic!("connect {addr}: {err}"));
    stream
        .set_nodelay(true)
        .unwrap_or_else(|err| panic!("nodelay: {err}"));
    stream
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
