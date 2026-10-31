use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;
use spookery::gm_connection::{Connection, Incoming};
use spookery::protocol;
use spookery_engine::testing::journal::list_round_keys;
use spookery_engine::State;

use crate::support::{self, Process};
use crate::World;

pub struct Seat {
    pub actor_id: String,
    pub write: TcpStream,
    pub text: Arc<Mutex<String>>,
}

pub struct Host {
    pub game_port: u16,
    pub first_stream: Option<TcpStream>,
    pub player_text: Arc<Mutex<String>>,
    pub players: Vec<Seat>,
    pub gm: Option<Connection>,
    /// The GM connection that called `close` and has not been dropped.
    pub closed_gm: Option<Connection>,
    /// Set when `read_presence` opened a GM only to read seats.
    pub gm_probe: bool,
    pub door_port: u16,
    pub gm_text: Arc<Mutex<String>>,
    pub opened_save: Vec<u8>,
    pub seated_save: Vec<u8>,
    pub seated_rev: u64,
    pub host: Option<Process>,
    /// A host `serve` runs in this process. Dropped last, so seats close first.
    pub in_process: Option<InProcess>,
}

type Advance = (Duration, tokio::sync::oneshot::Sender<()>);

/// `serve` on its own thread, on a current-thread runtime whose clock starts
/// paused. The table's blocking task keeps tokio from auto-advancing, so
/// paused time moves only on `advance`.
pub struct InProcess {
    pub stdout: Arc<Mutex<String>>,
    pub stderr: Arc<Mutex<String>>,
    advance: tokio::sync::mpsc::UnboundedSender<Advance>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for InProcess {
    /// Resolve the shutdown future and wait, so the pack lock is free after.
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Opens the pack the way `spook run` does and serves it in this process.
pub fn serve_in_process(world: &mut World, pack: &Path) {
    drop(world.live.take());
    // `lock_pack` then `World::resume`, as `spook run` opens it.
    let opened = support::resume(pack);
    let (mut out, out_pipe) = std::io::pipe().unwrap_or_else(|err| panic!("pipe: {err}"));
    let (err, err_pipe) = std::io::pipe().unwrap_or_else(|err| panic!("pipe: {err}"));
    let stderr = Arc::new(Mutex::new(String::new()));
    drop(drain(err, Arc::clone(&stderr)));
    let loopback: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let mut config = spookery_server::Config::new(loopback);
    config.door = Some(loopback);
    config.out = Box::new(out_pipe);
    config.err = Box::new(err_pipe);
    let (advance, mut advances) = tokio::sync::mpsc::unbounded_channel::<Advance>();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let failed = Arc::clone(&stderr);
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap_or_else(|err| panic!("runtime: {err}"));
        runtime.block_on(async move {
            tokio::spawn(async move {
                while let Some((by, ack)) = advances.recv().await {
                    tokio::time::advance(by).await;
                    let _ = ack.send(());
                }
            });
            let shutdown = async {
                let _ = stopped.await;
            };
            if let Err(err) = spookery_server::serve(opened, config, shutdown).await {
                failed.lock().unwrap().push_str(&format!("serve: {err}\n"));
            }
        });
    });
    let (game, door, seen) = match read_addrs(&mut out) {
        Ok(report) => report,
        Err(got) => {
            let _ = thread.join();
            let stderr = support::text_of(&stderr);
            panic!("serve ended before it reported its ports\nstdout:\n{got}\nstderr:\n{stderr}");
        }
    };
    let stdout = Arc::new(Mutex::new(seen));
    drop(drain(out, Arc::clone(&stdout)));
    attach_host(world, pack, game, door).in_process = Some(InProcess {
        stdout,
        stderr,
        advance,
        stop: Some(stop),
        thread: Some(thread),
    });
}

/// Moves the in-process host's paused clock forward. Timers due by then have
/// fired when this returns; what they sent the table is queued, so the next
/// `read_presence` is the barrier before reading the save or journal.
pub fn advance(world: &World, by: Duration) {
    let host = live(world)
        .in_process
        .as_ref()
        .unwrap_or_else(|| panic!("the table is not in this process"));
    let ended = || panic!("the host ended\n{}", support::text_of(&host.stderr));
    let (ack, acked) = tokio::sync::oneshot::channel();
    host.advance.send((by, ack)).unwrap_or_else(|_| ended());
    support::block_on(acked).unwrap_or_else(|_| ended());
}

pub fn live_mut(world: &mut World) -> &mut Host {
    world
        .live
        .as_mut()
        .unwrap_or_else(|| panic!("no table is listening"))
}

fn live(world: &World) -> &Host {
    world
        .live
        .as_ref()
        .unwrap_or_else(|| panic!("no table is listening"))
}

pub fn read_files(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
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
            } else if path
                .file_name()
                .is_none_or(|n| n != ".pack.lock" && n != ".pack.rw.lock")
            {
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
pub fn now_of(pack: &Path) -> State {
    let start = Instant::now();
    loop {
        match spookery_engine::testing::replay_journal(pack, None, None) {
            Ok(report) => {
                let text = std::str::from_utf8(&report.now_yaml)
                    .unwrap_or_else(|err| panic!("save text: {err}"));
                return spookery_engine::parse_save(text)
                    .unwrap_or_else(|err| panic!("load now: {err:?}"));
            }
            Err(err) if start.elapsed() > Duration::from_secs(2) => {
                panic!("replay now: {err}");
            }
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// The actors whose player `wait` is in that round.
pub fn list_player_waits(pack: &Path, name: &str) -> Vec<String> {
    let text = spookery_engine::testing::journal::read_record_line(pack, name)
        .unwrap_or_else(|err| panic!("read {name}: {}", err.detail));
    let round: Value =
        serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("{name}: {err}"));
    round["intents"]
        .as_array()
        .unwrap_or_else(|| panic!("intents"))
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .map(|intent| {
            intent["actor"]
                .as_str()
                .and_then(|actor| actor.strip_prefix("player:"))
                .unwrap_or_else(|| panic!("actor"))
                .to_string()
        })
        .collect()
}

/// How long the journal may sit still before a wait calls it a hang. It bounds
/// one commit, not the whole wait, so a slow machine only makes the wait longer.
const JOURNAL_STALL: Duration = Duration::from_secs(10);

/// Waits until `done` holds for the committed round keys. Fails only when no
/// round commits for `JOURNAL_STALL`; `want` names the condition in the panic.
pub fn wait_rounds_until(pack: &Path, want: &str, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    let mut seen = 0;
    let mut moved = Instant::now();
    loop {
        let keys = list_round_keys(pack);
        if done(&keys) {
            return keys;
        }
        if keys.len() != seen {
            seen = keys.len();
            moved = Instant::now();
        } else if moved.elapsed() > JOURNAL_STALL {
            panic!("no round committed for {JOURNAL_STALL:?} at {seen} rounds; wanted {want}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until the journal holds at least `count` rounds.
pub fn wait_round_count(pack: &Path, count: usize) -> Vec<String> {
    wait_rounds_until(pack, &format!("{count} rounds"), |keys| keys.len() >= count)
}

/// Waits until a round after `before` holds that actor's player `wait`.
pub fn wait_committed_wait(pack: &Path, actor_id: &str, before: usize) -> Vec<String> {
    wait_rounds_until(pack, &format!("{actor_id}'s wait"), |keys| {
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

pub struct RunningTable {
    pub game: SocketAddr,
    pub door: SocketAddr,
    pub host: Process,
}

pub fn listen(world: &mut World, pack: &Path) {
    let _ = run_with_flags(world, pack, &["--addr", "127.0.0.1:0"]);
}

/// `spook run` with an agent door the operating system chooses on its own.
pub fn listen_door(world: &mut World, pack: &Path) -> u16 {
    run_with_flags(
        world,
        pack,
        &["--addr", "127.0.0.1:0", "--door", "127.0.0.1:0"],
    )
    .1
}

pub fn run_with_flags(world: &mut World, pack: &Path, flags: &[&str]) -> (u16, u16) {
    drop(world.live.take());
    let addrs = wait_addrs(spawn_table(pack, flags));
    let ports = (addrs.game.port(), addrs.door.port());
    attach_host(world, pack, addrs.game, addrs.door).host = Some(addrs.host);
    ports
}

pub fn game_addr(world: &World) -> String {
    format!("127.0.0.1:{}", live(world).game_port)
}

pub fn door_addr(world: &World) -> String {
    format!("127.0.0.1:{}", live(world).door_port)
}

/// The listening line `spook run` wrote, as the addresses it bound.
pub fn parse_addrs(stdout: &str) -> (SocketAddr, SocketAddr) {
    stdout
        .lines()
        .find_map(parse_addrs_line)
        .unwrap_or_else(|| panic!("no listening line\n{stdout}"))
}

pub fn spawn_table(pack: &Path, flags: &[&str]) -> Pending {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.arg("run").arg(support::pack_arg(pack)).args(flags);
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

pub fn wait_addrs(mut pending: Pending) -> RunningTable {
    let (game, door, seen) = expect_addrs(&mut pending);
    let stdout = Arc::new(Mutex::new(seen));
    drop(drain(pending.stdout, Arc::clone(&stdout)));
    RunningTable {
        game,
        door,
        host: Process {
            child: pending.child,
            stdin: pending.stdin,
            stdout,
            stderr: pending.stderr,
        },
    }
}

fn attach_host<'w>(
    world: &'w mut World,
    pack: &Path,
    game: SocketAddr,
    door: SocketAddr,
) -> &'w mut Host {
    let opened_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    drop(world.live.take());
    world.live.insert(Host {
        game_port: game.port(),
        first_stream: None,
        player_text: Arc::new(Mutex::new(String::new())),
        players: Vec::new(),
        gm: None,
        closed_gm: None,
        gm_probe: false,
        door_port: door.port(),
        gm_text: Arc::new(Mutex::new(String::new())),
        opened_save: opened_now.clone(),
        seated_save: opened_now,
        seated_rev: 0,
        host: None,
        in_process: None,
    })
}

fn expect_addrs(pending: &mut Pending) -> (SocketAddr, SocketAddr, String) {
    read_addrs(&mut pending.stdout).unwrap_or_else(|got| {
        let status = pending
            .child
            .wait()
            .unwrap_or_else(|err| panic!("wait: {err}"));
        if let Some(drain) = pending.stderr_drain.take() {
            let _ = drain.join();
        }
        panic!(
            "spook run exited {status} before it reported its ports\nstdout:\n{got}\nstderr:\n{}",
            support::text_of(&pending.stderr)
        );
    })
}

/// Reads the host's stdout up to its `listening` line: the two addresses and
/// the text so far. `Err` is the text before stdout closed without one.
fn read_addrs(stdout: &mut impl Read) -> Result<(SocketAddr, SocketAddr, String), String> {
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match stdout.read(&mut buf) {
            Ok(0) => return Err(String::from_utf8_lossy(&got).into_owned()),
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if let Some((game, door)) = find_addrs(&got) {
                    return Ok((game, door, String::from_utf8_lossy(&got).into_owned()));
                }
            }
            Err(err) => panic!("read stdout: {err}"),
        }
    }
}

fn find_addrs(buf: &[u8]) -> Option<(SocketAddr, SocketAddr)> {
    let end = buf.iter().rposition(|byte| *byte == b'\n')? + 1;
    let text = std::str::from_utf8(&buf[..end]).unwrap_or_else(|err| panic!("stdout: {err}"));
    text.lines().find_map(parse_addrs_line)
}

fn parse_addrs_line(line: &str) -> Option<(SocketAddr, SocketAddr)> {
    let addrs: protocol::Addrs = serde_json::from_str(line.trim()).ok()?;
    Some((addrs.game.parse().ok()?, addrs.door.parse().ok()?))
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

pub fn seat_rowan(world: &mut World, pack: &Path) {
    seat_actor(world, pack, "pc_rowan");
}

pub fn seat_actor(world: &mut World, pack: &Path, actor_id: &str) {
    let port = live(world).game_port;
    let (write, text) = connect_actor(port, actor_id);
    let seated_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    let seated_rev = now_of(pack).state_rev;
    let live = live_mut(world);
    if live.first_stream.is_none() {
        live.first_stream = Some(
            write
                .try_clone()
                .unwrap_or_else(|err| panic!("clone player: {err}")),
        );
        live.player_text = Arc::clone(&text);
    }
    live.players.push(Seat {
        actor_id: actor_id.to_string(),
        write,
        text,
    });
    live.seated_save = seated_now;
    live.seated_rev = seated_rev;
}

pub fn seat_gm(world: &mut World) {
    let door = door_addr(world).parse().unwrap();
    let gm = support::block_on(Connection::connect_door(door))
        .unwrap_or_else(|err| panic!("seat GM: {err}"));
    let opened = gm.initialize_line().to_string();
    let live = live_mut(world);
    push_line(&live.gm_text, &opened);
    live.gm = Some(gm);
}

pub fn player_mark(world: &World) -> usize {
    support::text_of(&live(world).player_text).len()
}

pub fn player_text(world: &World) -> String {
    support::text_of(&live(world).player_text)
}

/// The reply to a player `look`, which takes no round.
pub const LINES_KIND: &str = "\"kind\":\"lines\"";

pub fn player_send(world: &mut World, line: &str, needle: &str) -> (usize, String) {
    let mark = player_write(world, line);
    (mark, wait_player_from(world, mark, needle))
}

/// Sends one line from the first seated player and returns the text mark before it.
pub fn player_write(world: &mut World, line: &str) -> usize {
    let live = live_mut(world);
    let mark = support::text_of(&live.player_text).len();
    let write = live
        .first_stream
        .as_mut()
        .unwrap_or_else(|| panic!("no seated player"));
    writeln!(write, "{line}").unwrap_or_else(|err| panic!("write: {err}"));
    write.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    mark
}

pub fn wait_player_from(world: &World, mark: usize, needle: &str) -> String {
    let slot = Arc::clone(&live(world).player_text);
    wait_suffix(&slot, mark, needle)
}

pub fn wait_actor_from(world: &World, actor_id: &str, mark: usize, needle: &str) -> String {
    let slot = Arc::clone(&seat(world, actor_id).text);
    wait_suffix(&slot, mark, needle)
}
/// The seated player for `actor_id`.
fn seat<'w>(world: &'w World, actor_id: &str) -> &'w Seat {
    live(world)
        .players
        .iter()
        .find(|player| player.actor_id == actor_id)
        .unwrap_or_else(|| panic!("no seat for {actor_id}"))
}

pub fn gm_call(world: &mut World, id: u64, name: &str, arguments: &str) -> String {
    let live = live_mut(world);
    let gm = live.gm.as_mut().unwrap_or_else(|| panic!("no seated GM"));
    send_tool_call(gm, &live.gm_text, id, name, arguments)
}

/// A call from the GM that called `close` and kept its session.
pub fn closed_gm_call(world: &mut World, name: &str, arguments: &str) -> String {
    let live = live_mut(world);
    let gm = live
        .closed_gm
        .as_mut()
        .unwrap_or_else(|| panic!("no closed GM"));
    send_tool_call(gm, &live.gm_text, 92, name, arguments)
}

/// One tool call. Its events and its result land in the GM text.
fn send_tool_call(
    gm: &mut Connection,
    gm_text: &Arc<Mutex<String>>,
    id: u64,
    name: &str,
    arguments: &str,
) -> String {
    let value: Value =
        serde_json::from_str(arguments).unwrap_or_else(|err| panic!("arguments: {err}"));
    let outcome =
        support::block_on(gm.call(name, value)).unwrap_or_else(|err| panic!("gm call: {err}"));
    for event in gm.take_events() {
        push_line(gm_text, &event.raw);
    }
    let raw = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "structuredContent": outcome.body }
    })
    .to_string();
    push_line(gm_text, &raw);
    raw
}

pub fn wait_gm(world: &mut World, needle: &str) -> String {
    wait_gm_until(world, needle, |text| text.contains(needle))
}

/// Waits until `done` holds for the GM text; `want` names the condition in the panic.
pub fn wait_gm_until(world: &mut World, want: &str, done: impl Fn(&str) -> bool) -> String {
    poll_until(want, || {
        let text = drain_gm_text(world);
        if done(&text) {
            Ok(text)
        } else {
            Err(text)
        }
    })
}

pub fn wait_world_time(pack: &Path, expected: u64) {
    poll_until(&format!("world_time {expected}"), || {
        let now = now_of(pack).world_time;
        if now == expected {
            Ok(())
        } else {
            Err(now)
        }
    });
}

/// How long a wait gives the host to do what a step asked.
const WAIT: Duration = Duration::from_secs(5);

/// Calls `check` until it returns `Ok`. After `WAIT` it panics with `want` and
/// the last `Err`, which says what was seen instead.
fn poll_until<T, Seen: std::fmt::Debug>(
    want: &str,
    mut check: impl FnMut() -> Result<T, Seen>,
) -> T {
    let start = Instant::now();
    loop {
        match check() {
            Ok(done) => return done,
            Err(seen) if start.elapsed() > WAIT => panic!("missing {want}\n{seen:?}"),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

pub fn host_stdout(world: &World) -> String {
    let live = live(world);
    let process = live.host.as_ref().map(|server| &server.stdout);
    let in_process = live.in_process.as_ref().map(|server| &server.stdout);
    process
        .or(in_process)
        .map(support::text_of)
        .unwrap_or_default()
}

pub fn wait_host_until(world: &World, want: &str, done: impl Fn(&str) -> bool) -> String {
    poll_until(want, || {
        let text = host_stdout(world);
        if done(&text) {
            Ok(text)
        } else {
            Err(text)
        }
    })
}

pub fn drain_gm_text(world: &mut World) -> String {
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
        Incoming::Result { raw, .. } | Incoming::Invalid(raw) | Incoming::Refused { raw, .. } => {
            Some(raw)
        }
        Incoming::Eof => None,
    }
}

pub fn actor_text(world: &World, actor_id: &str) -> String {
    support::text_of(&seat(world, actor_id).text)
}

pub fn actor_send(world: &mut World, actor_id: &str, line: &str, needle: &str) -> String {
    let mark = actor_write(world, actor_id, line);
    wait_actor_from(world, actor_id, mark, needle)
}

pub fn actor_write(world: &mut World, actor_id: &str, line: &str) -> usize {
    let player = seat(world, actor_id);
    let mark = support::text_of(&player.text).len();
    // `&TcpStream` writes too, so a shared seat is enough.
    let mut write = &player.write;
    writeln!(write, "{line}").unwrap_or_else(|err| panic!("write: {err}"));
    write.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    mark
}

pub fn stop_host(world: &mut World) {
    let live = live_mut(world);
    live.players.clear();
    live.first_stream.take();
    live.gm.take();
    live.closed_gm.take();
    // An in-process host stops when its shutdown future resolves.
    live.in_process.take();
    // Stop the host the way a person does, with Ctrl+C. Windows just drops it, which kills.
    if let Some(mut server) = live.host.take() {
        if cfg!(unix) {
            let pid = server.child.id().to_string();
            let kill = std::process::Command::new("kill")
                .args(["-INT", &pid])
                .status();
            assert!(kill.is_ok_and(|s| s.success()), "kill -INT {pid}");
            let status = server
                .child
                .wait()
                .unwrap_or_else(|err| panic!("wait: {err}"));
            let stderr = support::text_of(&server.stderr);
            assert!(status.success(), "host exited {status:?}\n{stderr}");
        }
    }
}

pub fn close_actor(world: &mut World, actor_id: &str) {
    let _ = actor_write(world, actor_id, "close");
}

/// `close` answers after the seat is free. Dropping the socket does not.
pub fn release_gm(world: &mut World) -> String {
    close_gm(world, false)
}

/// `close`, keeping the session so a later call can be refused.
pub fn release_gm_keep_session(world: &mut World) -> String {
    close_gm(world, true)
}

fn close_gm(world: &mut World, keep_session: bool) -> String {
    let text = gm_call(world, 91, "close", "{}");
    assert!(
        text.contains("\"released\":true"),
        "the server did not confirm the seat was free\n{text}"
    );
    let live = live_mut(world);
    let gm = live.gm.take();
    if keep_session {
        live.closed_gm = gm;
    }
    live.gm_probe = false;
    text
}

pub fn release_probe_gm(world: &mut World) {
    if live(world).gm_probe {
        let _ = release_gm(world);
    }
}

pub fn read_presence(world: &mut World) -> (u32, Vec<String>) {
    let body = call_table_status(world);
    let players = body["presence"]["players"]
        .as_u64()
        .unwrap_or_else(|| panic!("players\n{body}")) as u32;
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
    (players, actors)
}

fn call_table_status(world: &mut World) -> Value {
    if live(world).gm.is_none() {
        let door = door_addr(world).parse().unwrap();
        let gm = support::block_on(Connection::connect_door(door))
            .unwrap_or_else(|err| panic!("status: {err}"));
        let live = live_mut(world);
        live.gm = Some(gm);
        live.gm_probe = true;
    }
    let text = gm_call(world, 1, "table_status", "{}");
    let value: Value =
        serde_json::from_str(&text).unwrap_or_else(|err| panic!("status json: {err}"));
    value["result"]["structuredContent"].clone()
}

pub fn wait_players(world: &mut World, expected: u32) {
    wait_presence(world, &format!("players {expected}"), |players, _| {
        players == expected
    });
}

/// Waits until the GM status presence passes `done`; `want` names it in the panic.
pub fn wait_presence(world: &mut World, want: &str, done: impl Fn(u32, &[String]) -> bool) {
    poll_until(want, || {
        let (players, actors) = read_presence(world);
        if done(players, &actors) {
            Ok(())
        } else {
            Err((players, actors))
        }
    });
}

pub fn send_hello(world: &World, line: &str) -> String {
    let port = live(world).game_port;
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
    wait_text(&text, "\"kind\":\"view\"");
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

pub fn wait_suffix(slot: &Arc<Mutex<String>>, mark: usize, needle: &str) -> String {
    poll_until(needle, || {
        let text = support::text_of(slot);
        let added = text.get(mark..).unwrap_or("").to_string();
        if added.contains(needle) {
            Ok(added)
        } else {
            Err(added)
        }
    })
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
    let output = support::run_spook(args);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}
