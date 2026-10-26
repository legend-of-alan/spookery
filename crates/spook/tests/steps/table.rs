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
use spookery_engine::model::Now;
use spookery_engine::testing::journal::list_tick_keys;

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
    /// The GM connection that called `close` and has not been dropped.
    pub closed_gm: Option<GmConnection>,
    /// Set when `live_presence` opened a GM only to read seats.
    pub gm_probe: bool,
    pub http_port: u16,
    pub gm_text: Arc<Mutex<String>>,
    pub opened_now: Vec<u8>,
    pub seated_now: Vec<u8>,
    pub seated_rev: u64,
    pub server: Option<Proc>,
}

pub fn live_mut(world: &mut World) -> &mut Live {
    world
        .live
        .as_mut()
        .unwrap_or_else(|| panic!("no table is listening"))
}

fn live_ref(world: &World) -> &Live {
    world
        .live
        .as_ref()
        .unwrap_or_else(|| panic!("no table is listening"))
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
            } else if path.file_name().is_none_or(|n| n != ".serve.lock") {
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

/// Replay the journal to the current state. `save.yaml` is the last checkpoint.
/// A torn last line is ignored, so a read during an append still sees complete
/// records. A failed read retries.
pub fn now_of(pack: &Path) -> Now {
    let start = Instant::now();
    loop {
        match spookery_engine::testing::from_journal(pack, None, None) {
            Ok(report) => {
                let text = std::str::from_utf8(&report.now_yaml)
                    .unwrap_or_else(|err| panic!("save text: {err}"));
                return spookery_engine::store::load_save(text)
                    .unwrap_or_else(|err| panic!("load now: {err:?}"));
            }
            Err(err) if start.elapsed() > Duration::from_secs(2) => {
                panic!("replay now: {err}");
            }
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// The actors whose player `wait` is in that tick.
pub fn list_player_waits(pack: &Path, name: &str) -> Vec<String> {
    let text = spookery_engine::testing::journal::read_record_line(pack, name)
        .unwrap_or_else(|err| panic!("read {name}: {}", err.detail));
    let tick: Value =
        serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("{name}: {err}"));
    tick["intents"]
        .as_array()
        .unwrap_or_else(|| panic!("intents"))
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .map(|intent| {
            intent["actor_id"]
                .as_str()
                .unwrap_or_else(|| panic!("actor_id"))
                .to_string()
        })
        .collect()
}

/// How long the journal may sit still before a wait calls it a hang. It bounds
/// one commit, not the whole wait, so a slow machine only makes the wait longer.
const JOURNAL_STALL: Duration = Duration::from_secs(10);

/// Waits until `done` holds for the committed tick keys. Fails only when no
/// tick commits for `JOURNAL_STALL`; `want` names the condition in the panic.
pub fn wait_ticks_until(pack: &Path, want: &str, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    let mut seen = 0;
    let mut moved = Instant::now();
    loop {
        let keys = list_tick_keys(pack);
        if done(&keys) {
            return keys;
        }
        if keys.len() != seen {
            seen = keys.len();
            moved = Instant::now();
        } else if moved.elapsed() > JOURNAL_STALL {
            panic!("no tick committed for {JOURNAL_STALL:?} at {seen} ticks; wanted {want}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until the journal holds at least `count` ticks.
pub fn wait_tick_count(pack: &Path, count: usize) -> Vec<String> {
    wait_ticks_until(pack, &format!("{count} ticks"), |keys| keys.len() >= count)
}

/// Waits until a tick after `before` holds that actor's player `wait`.
pub fn expect_wait_from(pack: &Path, actor_id: &str, before: usize) -> Vec<String> {
    wait_ticks_until(pack, &format!("{actor_id}'s wait"), |keys| {
        keys.get(before..)
            .unwrap_or_default()
            .iter()
            .any(|key| list_player_waits(pack, key).iter().any(|id| id == actor_id))
    })
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

pub fn listen(world: &mut World, pack: &Path) {
    let _ = listen_flags(world, pack, &["--addr", "127.0.0.1:0"], &[]);
}

/// `spook run` with an agent door the operating system chooses on its own.
pub fn listen_door(world: &mut World, pack: &Path) -> u16 {
    listen_flags(
        world,
        pack,
        &["--addr", "127.0.0.1:0", "--mcp-http", "127.0.0.1:0"],
        &[],
    )
    .1
}

pub fn listen_with(world: &mut World, pack: &Path, env: &[(&str, &str)]) {
    let _ = listen_flags(world, pack, &["--addr", "127.0.0.1:0"], env);
}

pub fn listen_flags(
    world: &mut World,
    pack: &Path,
    flags: &[&str],
    env: &[(&str, &str)],
) -> (u16, u16) {
    drop(world.live.take());
    let listening = take_report(spawn_table(pack, flags, env));
    let ports = (listening.game.port(), listening.door.port());
    install(world, pack, listening);
    ports
}

pub fn game_addr(world: &World) -> String {
    format!("127.0.0.1:{}", live_ref(world).port)
}

pub fn door_addr(world: &World) -> String {
    format!("127.0.0.1:{}", live_ref(world).http_port)
}

/// The listening line `spook run` wrote, as the addresses it bound.
pub fn reported(stdout: &str) -> (SocketAddr, SocketAddr) {
    stdout
        .lines()
        .find_map(parse_line)
        .unwrap_or_else(|| panic!("no listening line\n{stdout}"))
}

pub fn spawn_table(pack: &Path, flags: &[&str], env: &[(&str, &str)]) -> Pending {
    spawn_host(pack, flags, env, false)
}

pub fn spawn_host(
    pack: &Path,
    flags: &[&str],
    env: &[(&str, &str)],
    closed_stdin: bool,
) -> Pending {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.arg("run").arg(support::pack_arg(pack)).args(flags);
    for (key, value) in env {
        command.env(key, value);
    }
    // Windows can aim Ctrl+Break only at a process group created this way.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    command
        .stdin(if closed_stdin {
            Stdio::null()
        } else {
            Stdio::piped()
        })
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

pub(crate) fn install(world: &mut World, pack: &Path, listening: Listening) {
    let opened_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    drop(world.live.take());
    world.live = Some(Live {
        port: listening.game.port(),
        player_write: None,
        player_text: Arc::new(Mutex::new(String::new())),
        players: Vec::new(),
        gm: None,
        closed_gm: None,
        gm_probe: false,
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

pub fn seat_player(world: &mut World, pack: &Path) {
    seat_actor(world, pack, "pc_rowan");
}

pub fn seat_actor(world: &mut World, pack: &Path, actor_id: &str) {
    let port = live_ref(world).port;
    let (write, text) = connect_actor(port, actor_id);
    let seated_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    let seated_rev = now_of(pack).state_rev;
    let live = live_mut(world);
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
}

pub fn seat_gm(world: &mut World) {
    let door = door_addr(world).parse().unwrap();
    let gm = GmConnection::connect_door(door).unwrap_or_else(|err| panic!("seat GM: {err}"));
    let opened = gm.opened_line().to_string();
    let live = live_mut(world);
    push_line(&live.gm_text, &opened);
    live.gm = Some(gm);
}

pub fn player_len(world: &World) -> usize {
    support::text_of(&live_ref(world).player_text).len()
}

pub fn player_text(world: &World) -> String {
    support::text_of(&live_ref(world).player_text)
}

pub fn player_send(world: &mut World, line: &str, needle: &str) -> (usize, String) {
    let mark = player_write(world, line);
    (mark, wait_player_from(world, mark, needle))
}

/// Sends one line from the first seated player and returns the text mark before it.
pub fn player_write(world: &mut World, line: &str) -> usize {
    let live = live_mut(world);
    let mark = support::text_of(&live.player_text).len();
    let write = live
        .player_write
        .as_mut()
        .unwrap_or_else(|| panic!("no seated player"));
    writeln!(write, "{line}").unwrap_or_else(|err| panic!("write: {err}"));
    write.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    mark
}

pub fn wait_player_from(world: &World, mark: usize, needle: &str) -> String {
    let slot = Arc::clone(&live_ref(world).player_text);
    wait_suffix(&slot, mark, needle)
}

pub fn wait_actor_from(world: &World, actor_id: &str, mark: usize, needle: &str) -> String {
    let slot = Arc::clone(
        &live_ref(world)
            .players
            .iter()
            .find(|player| player.actor_id == actor_id)
            .unwrap_or_else(|| panic!("no seat for {actor_id}"))
            .text,
    );
    wait_suffix(&slot, mark, needle)
}

pub fn gm_call(world: &mut World, id: u64, name: &str, arguments: &str) -> String {
    let value: Value =
        serde_json::from_str(arguments).unwrap_or_else(|err| panic!("arguments: {err}"));
    let live = live_mut(world);
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
}

pub fn wait_gm(world: &mut World, needle: &str) -> String {
    wait_gm_until(world, needle, |text| text.contains(needle))
}

/// Waits until `done` holds for the GM text; `want` names the condition in the panic.
pub fn wait_gm_until(world: &mut World, want: &str, done: impl Fn(&str) -> bool) -> String {
    let start = Instant::now();
    loop {
        let text = gm_text(world);
        if done(&text) {
            return text;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("missing {want}\n{text}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_world_time(pack: &Path, expected: u64) {
    let start = Instant::now();
    loop {
        if now_of(pack).world_time == expected {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("world_time did not become {expected}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn host_stdout(world: &World) -> String {
    live_ref(world)
        .server
        .as_ref()
        .map(|server| support::text_of(&server.stdout))
        .unwrap_or_default()
}

pub fn wait_host_until(world: &World, want: &str, done: impl Fn(&str) -> bool) -> String {
    let start = Instant::now();
    loop {
        let text = host_stdout(world);
        if done(&text) {
            return text;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("missing {want}\n{text}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn gm_text(world: &mut World) -> String {
    let live = live_mut(world);
    if let Some(gm) = live.gm.as_mut() {
        let ready = gm.drain_ready();
        for incoming in ready {
            if let Some(line) = incoming_line(incoming) {
                push_line(&live.gm_text, &line);
            }
        }
    }
    support::text_of(&live.gm_text)
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

pub fn actor_text(world: &World, actor_id: &str) -> String {
    let slot = Arc::clone(
        &live_ref(world)
            .players
            .iter()
            .find(|player| player.actor_id == actor_id)
            .unwrap_or_else(|| panic!("no seat for {actor_id}"))
            .text,
    );
    support::text_of(&slot)
}

pub fn actor_send(world: &mut World, actor_id: &str, line: &str, needle: &str) -> String {
    let mark = actor_line(world, actor_id, line);
    let slot = Arc::clone(
        &live_ref(world)
            .players
            .iter()
            .find(|player| player.actor_id == actor_id)
            .unwrap_or_else(|| panic!("no seat for {actor_id}"))
            .text,
    );
    wait_suffix(&slot, mark, needle)
}

pub fn actor_line(world: &mut World, actor_id: &str, line: &str) -> usize {
    let live = live_mut(world);
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
    mark
}

pub fn stop_server(world: &mut World) {
    let live = live_mut(world);
    live.players.clear();
    live.player_write.take();
    live.gm.take();
    live.closed_gm.take();
    if let Some(server) = live.server.take() {
        drop(server);
    }
}

pub fn close_actor(world: &mut World, actor_id: &str) {
    let live = live_mut(world);
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
}

/// `close` answers after the seat is free. The session stays up so a later call can be refused.
pub fn release_gm_session(world: &mut World) -> String {
    let text = gm_call(world, 91, "close", "{}");
    assert!(
        text.contains("\"released\":true"),
        "the server did not confirm the seat was free\n{text}"
    );
    let live = live_mut(world);
    live.closed_gm = live.gm.take();
    live.gm_probe = false;
    text
}

pub fn closed_gm_call(world: &mut World, name: &str, arguments: &str) -> String {
    let value: Value =
        serde_json::from_str(arguments).unwrap_or_else(|err| panic!("arguments: {err}"));
    let live = live_mut(world);
    let gm = live
        .closed_gm
        .as_mut()
        .unwrap_or_else(|| panic!("no closed GM"));
    let outcome = gm
        .call(name, value)
        .unwrap_or_else(|err| panic!("gm call: {err}"));
    let raw = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 92,
        "result": { "structuredContent": outcome.body }
    })
    .to_string();
    push_line(&live.gm_text, &raw);
    raw
}

/// `close` answers after the seat is free. Dropping the socket does not.
pub fn release_gm(world: &mut World) -> String {
    let text = gm_call(world, 91, "close", "{}");
    assert!(
        text.contains("\"released\":true"),
        "the server did not confirm the seat was free\n{text}"
    );
    let live = live_mut(world);
    live.gm = None;
    live.gm_probe = false;
    text
}

pub fn release_status_gm(world: &mut World) {
    if live_ref(world).gm_probe {
        let _ = release_gm(world);
    }
}

pub fn live_presence(world: &mut World) -> (u32, Vec<String>) {
    let body = status_body(world);
    let humans = body["presence"]["humans"]
        .as_u64()
        .unwrap_or_else(|| panic!("humans\n{body}")) as u32;
    let actors = body["presence"]["actors_present"]
        .as_array()
        .unwrap_or_else(|| panic!("actors\n{body}"))
        .iter()
        .map(|id| {
            id.as_str()
                .unwrap_or_else(|| panic!("actor\n{body}"))
                .to_string()
        })
        .collect();
    (humans, actors)
}

fn status_body(world: &mut World) -> Value {
    if live_ref(world).gm.is_none() {
        let door = door_addr(world).parse().unwrap();
        let gm = GmConnection::connect_door(door).unwrap_or_else(|err| panic!("status: {err}"));
        let live = live_mut(world);
        live.gm = Some(gm);
        live.gm_probe = true;
    }
    let text = gm_call(world, 1, "session_status", "{}");
    let value: Value =
        serde_json::from_str(&text).unwrap_or_else(|err| panic!("status json: {err}"));
    value["result"]["structuredContent"].clone()
}

pub fn wait_humans(world: &mut World, _pack: &Path, expected: u32) {
    let start = Instant::now();
    loop {
        let (humans, _) = live_presence(world);
        if humans == expected {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("humans did not become {expected}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn second_hello(world: &World, line: &str) -> String {
    let port = live_ref(world).port;
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
