use std::fs;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use spookery::mcp::{event_line, GmClient};
use spookery::protocol::{self, ClientMessage};
use spookery_engine::testing::journal::list_round_keys;
use spookery_engine::testing::state_of;
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

use crate::support::{self, Process};
use crate::World;

/// One WebSocket connection to the game port's `/player` or `/gm`. A reader thread
/// appends each message the table sends to `text`, one line each.
pub struct Seat {
    write: Mutex<WebSocket<TcpStream>>,
    /// A seat that stops reading keeps its reading socket here, unread.
    quiet: Option<WebSocket<TcpStream>>,
    pub text: Arc<Mutex<String>>,
    /// Set once the table closed the connection.
    pub closed: Arc<AtomicBool>,
    next_id: AtomicU64,
}

/// Hangs up, so the reader ends and the table sees the connection close.
impl Drop for Seat {
    fn drop(&mut self) {
        let write = self.write.lock().unwrap_or_else(|err| err.into_inner());
        let _ = write.get_ref().shutdown(std::net::Shutdown::Both);
    }
}

impl Seat {
    /// Dials `/seat` on the game port and starts the reader. The hello is the first line.
    pub fn open(port: u16, seat: &str) -> Self {
        let stream = TcpStream::connect(("127.0.0.1", port))
            .unwrap_or_else(|err| panic!("connect {port}: {err}"));
        Self::from_stream(stream, port, seat, true)
    }

    /// Opens `/seat` over a connected `stream`. Without `reading`, nothing reads what the
    /// table sends.
    pub fn from_stream(stream: TcpStream, port: u16, seat: &str, reading: bool) -> Self {
        stream
            .set_nodelay(true)
            .unwrap_or_else(|err| panic!("nodelay: {err}"));
        let write = stream
            .try_clone()
            .unwrap_or_else(|err| panic!("clone: {err}"));
        let url = format!("ws://127.0.0.1:{port}/{seat}");
        let (mut read, _) =
            tungstenite::client(url, stream).unwrap_or_else(|err| panic!("{seat} seat: {err}"));
        let text = Arc::new(Mutex::new(String::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let (slot, ended) = (Arc::clone(&text), Arc::clone(&closed));
        let write = Mutex::new(WebSocket::from_raw_socket(write, Role::Client, None));
        if !reading {
            return Self {
                write,
                quiet: Some(read),
                text,
                closed,
                next_id: AtomicU64::new(1),
            };
        }
        // The handshake's socket reads, so a message sent with the upgrade is kept. A second
        // socket on the same stream only writes.
        std::thread::spawn(move || {
            loop {
                match read.read() {
                    Ok(Message::Text(line)) => push_line(&slot, &line),
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            ended.store(true, Ordering::SeqCst);
        });
        Self {
            write,
            quiet: None,
            text,
            closed,
            next_id: AtomicU64::new(1),
        }
    }

    /// Sends `word` with `args` under the next id, and returns that id.
    pub fn send(&self, word: &str, args: Value) -> String {
        let id = format!("t{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let message = ClientMessage::new(&id, word, args);
        let text = serde_json::to_string(&message).unwrap_or_else(|err| panic!("{word}: {err}"));
        self.send_raw(Message::text(text));
        id
    }

    /// Sends one WebSocket message as it is.
    pub fn send_raw(&self, message: Message) {
        self.try_send_raw(message)
            .unwrap_or_else(|err| panic!("send: {err}"));
    }

    /// Sends one WebSocket message as it is, or the error a closed connection gives.
    pub fn try_send_raw(&self, message: Message) -> tungstenite::Result<()> {
        let mut write = self.write.lock().unwrap_or_else(|err| err.into_inner());
        write.send(message)
    }

    /// Waits until the table has closed this connection.
    pub fn wait_closed(&self) {
        poll_until("the table to close the connection", || {
            self.closed.load(Ordering::SeqCst).then_some(()).ok_or(())
        });
    }

    pub fn mark(&self) -> usize {
        support::text_of(&self.text).len()
    }

    /// Waits for the reply to the message `id` named, and returns its line.
    pub fn wait_reply(&self, id: &str) -> String {
        let tag = format!("\"reply_to\":\"{id}\"");
        poll_until(&format!("the reply to {id}"), || {
            let text = support::text_of(&self.text);
            text.lines()
                .find(|line| line.contains(&tag))
                .map(str::to_string)
                .ok_or(text)
        })
    }
}

/// The word a typed line names and its arguments, as `spook play` sends it: `say <text>`
/// speaks to the table, and any other first word is a move with the rest as its target.
pub fn parse_typed(line: &str) -> (&str, Value) {
    match line.split_once(' ') {
        Some(("say", text)) => ("say", json!({ "address": "table", "text": text })),
        Some((word, target)) => (word, json!({ "target": target })),
        None => (line, json!({})),
    }
}

pub struct PlayerClient {
    pub actor_id: String,
    pub seat: Seat,
}

impl PlayerClient {
    pub fn text(&self) -> &Arc<Mutex<String>> {
        &self.seat.text
    }
}

pub struct Host {
    pub game_port: u16,
    pub players: Vec<PlayerClient>,
    pub gm: Option<GmClient>,
    /// A GM seated on `/gm` on the game port.
    pub ws_gm: Option<Seat>,
    /// A player connection a step opened and did not claim with.
    pub unclaimed: Option<Seat>,
    /// The GM client that called `release` and has not been dropped.
    pub closed_gm: Option<GmClient>,
    /// Set when `read_presence` opened a GM only to read seats.
    pub gm_probe: bool,
    pub mcp_port: u16,
    pub gm_text: Arc<Mutex<String>>,
    pub opened_save: Vec<u8>,
    pub seated_save: Vec<u8>,
    pub seated_rev: u64,
    pub host: Option<Process>,
    /// A host `serve` runs in this process. Dropped last, so seats close first.
    pub in_process: Option<InProcess>,
}

/// Sent from a step to a thread whose runtime's clock is paused: how far to move it, and the
/// reply once it has moved.
pub type Advance = (Duration, tokio::sync::oneshot::Sender<()>);

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
    drop(support::drain(err, Arc::clone(&stderr)));
    let loopback: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let mut config = spookery_server::Config::new(loopback);
    config.mcp_addr = Some(loopback);
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
    let (game, mcp_addr, seen) = match read_addrs(&mut out) {
        Ok(report) => report,
        Err(got) => {
            let _ = thread.join();
            let stderr = support::text_of(&stderr);
            panic!("serve ended before it reported its ports\nstdout:\n{got}\nstderr:\n{stderr}");
        }
    };
    let stdout = Arc::new(Mutex::new(seen));
    drop(support::drain(out, Arc::clone(&stdout)));
    attach_host(world, pack, game, mcp_addr).in_process = Some(InProcess {
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
    pub mcp_addr: SocketAddr,
    pub host: Process,
}

pub fn listen(world: &mut World, pack: &Path) {
    let _ = run_with_flags(world, pack, &["--addr", "127.0.0.1:0"]);
}

/// `spook run` with an MCP address the operating system chooses on its own.
pub fn listen_on_free_ports(world: &mut World, pack: &Path) -> u16 {
    run_with_flags(
        world,
        pack,
        &["--addr", "127.0.0.1:0", "--mcp-addr", "127.0.0.1:0"],
    )
    .1
}

pub fn run_with_flags(world: &mut World, pack: &Path, flags: &[&str]) -> (u16, u16) {
    drop(world.live.take());
    let addrs = wait_addrs(spawn_table(pack, flags));
    let ports = (addrs.game.port(), addrs.mcp_addr.port());
    attach_host(world, pack, addrs.game, addrs.mcp_addr).host = Some(addrs.host);
    ports
}

pub fn game_addr(world: &World) -> String {
    format!("127.0.0.1:{}", live(world).game_port)
}

pub fn mcp_addr(world: &World) -> String {
    format!("127.0.0.1:{}", live(world).mcp_port)
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
    let stderr_drain = support::drain(stderr_pipe, Arc::clone(&stderr));
    Pending {
        child,
        stdin,
        stdout,
        stderr,
        stderr_drain: Some(stderr_drain),
    }
}

pub fn wait_addrs(mut pending: Pending) -> RunningTable {
    let (game, mcp_addr, seen) = expect_addrs(&mut pending);
    let stdout = Arc::new(Mutex::new(seen));
    drop(support::drain(pending.stdout, Arc::clone(&stdout)));
    RunningTable {
        game,
        mcp_addr,
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
    mcp_addr: SocketAddr,
) -> &'w mut Host {
    let opened_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    drop(world.live.take());
    world.live.insert(Host {
        game_port: game.port(),
        players: Vec::new(),
        gm: None,
        ws_gm: None,
        unclaimed: None,
        closed_gm: None,
        gm_probe: false,
        mcp_port: mcp_addr.port(),
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
                if let Some((game, mcp_addr)) = find_addrs(&got) {
                    return Ok((game, mcp_addr, String::from_utf8_lossy(&got).into_owned()));
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
    Some((addrs.game.parse().ok()?, addrs.mcp.parse().ok()?))
}

pub fn seat_rowan(world: &mut World, pack: &Path) {
    seat_actor(world, pack, "pc_rowan");
}

pub fn seat_actor(world: &mut World, pack: &Path, actor_id: &str) {
    let port = live(world).game_port;
    let seat = connect_actor(port, actor_id);
    let seated_now = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    let seated_rev = state_of(pack).state_rev;
    let live = live_mut(world);
    live.players.push(PlayerClient {
        actor_id: actor_id.to_string(),
        seat,
    });
    live.seated_save = seated_now;
    live.seated_rev = seated_rev;
}

pub fn seat_gm(world: &mut World) {
    let mcp_addr = mcp_addr(world).parse().unwrap();
    let gm = support::block_on(GmClient::connect(mcp_addr))
        .unwrap_or_else(|err| panic!("seat GM: {err}"));
    live_mut(world).gm = Some(gm);
}

/// The first seated player.
fn first_player(world: &World) -> &PlayerClient {
    live(world)
        .players
        .first()
        .unwrap_or_else(|| panic!("no seated player"))
}

pub fn player_mark(world: &World) -> usize {
    first_player(world).seat.mark()
}

pub fn player_text(world: &World) -> String {
    support::text_of(first_player(world).text())
}

/// Sends one typed line from the first seated player and waits for its reply. Returns the
/// text mark before the line and the text added through `needle`.
pub fn player_send(world: &mut World, line: &str, needle: &str) -> (usize, String) {
    let actor_id = first_player(world).actor_id.clone();
    let mark = player_mark(world);
    (mark, actor_send(world, &actor_id, line, needle))
}

/// Sends one typed line from the first seated player and returns the text mark before it.
pub fn player_write(world: &mut World, line: &str) -> usize {
    let actor_id = first_player(world).actor_id.clone();
    actor_write(world, &actor_id, line)
}

/// The world times of the rounds the first player hears after `mark`, read once a view
/// follows them. One commit sends all its rounds before any player's view.
pub fn wait_rounds_to_view(world: &World, mark: usize) -> Vec<u64> {
    let text = first_player(world).text();
    poll_until("a round and the view after it", || {
        let added = support::text_of(text)
            .get(mark..)
            .unwrap_or_default()
            .to_string();
        let mut rounds = Vec::new();
        for line in added.lines() {
            let Ok(message) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match message["body"]["kind"].as_str() {
                Some("round") => rounds.extend(message["body"]["world_time"].as_u64()),
                Some("view") if !rounds.is_empty() => return Ok(rounds),
                _ => {}
            }
        }
        Err(added)
    })
}

pub fn wait_player_from(world: &World, mark: usize, needle: &str) -> String {
    wait_suffix(first_player(world).text(), mark, needle)
}

pub fn wait_actor_from(world: &World, actor_id: &str, mark: usize, needle: &str) -> String {
    wait_suffix(player_client(world, actor_id).text(), mark, needle)
}

/// The seated player for `actor_id`.
pub fn player_client<'w>(world: &'w World, actor_id: &str) -> &'w PlayerClient {
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

/// A call from the GM that called `release` and kept its session.
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
    gm: &mut GmClient,
    gm_text: &Arc<Mutex<String>>,
    id: u64,
    word: &str,
    arguments: &str,
) -> String {
    let value: Value =
        serde_json::from_str(arguments).unwrap_or_else(|err| panic!("arguments: {err}"));
    let messages =
        support::block_on(gm.call(word, &value)).unwrap_or_else(|err| panic!("gm call: {err}"));
    for event in gm.drain_events() {
        push_line(gm_text, &event_line(&event));
    }
    let body = json!(protocol::ToolResult { messages });
    let raw = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "structuredContent": body }
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

/// Waits until a seated player or the WebSocket GM hears the round that ends at
/// `world_time`, and returns that line.
pub fn wait_round(world: &World, world_time: u64) -> String {
    let tag = format!("\"world_time\":{world_time}}}");
    let live = live(world);
    let texts: Vec<&Arc<Mutex<String>>> = live
        .players
        .iter()
        .map(PlayerClient::text)
        .chain(live.ws_gm.as_ref().map(|gm| &gm.text))
        .collect();
    poll_until(&format!("the round at world_time {world_time}"), || {
        let mut seen = String::new();
        for text in &texts {
            let text = support::text_of(text);
            if let Some(line) = text
                .lines()
                .find(|line| line.contains("\"kind\":\"round\"") && line.contains(&tag))
            {
                return Ok(line.to_string());
            }
            seen.push_str(&text);
        }
        Err(seen)
    })
}

/// How long a wait gives the host to do what a step asked.
const WAIT: Duration = Duration::from_secs(5);

/// Calls `check` until it returns `Ok`. After `WAIT` it panics with `want` and
/// the last `Err`, which says what was seen instead.
pub fn poll_until<T, Seen: std::fmt::Debug>(
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
        for received in gm.drain() {
            let line = match received {
                Ok(event) => event_line(&event),
                Err(err) => err.to_string(),
            };
            push_line(&live.gm_text, &line);
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

pub fn actor_text(world: &World, actor_id: &str) -> String {
    support::text_of(player_client(world, actor_id).text())
}

/// Sends one typed line from `actor_id`, waits for its reply, then for `needle` after the
/// mark. Returns the text added since the mark.
pub fn actor_send(world: &mut World, actor_id: &str, line: &str, needle: &str) -> String {
    let player = player_client(world, actor_id);
    let mark = player.seat.mark();
    let (word, args) = parse_typed(line);
    let id = player.seat.send(word, args);
    player.seat.wait_reply(&id);
    wait_actor_from(world, actor_id, mark, needle)
}

/// Sends one typed line from `actor_id` and returns the text mark before it.
pub fn actor_write(world: &mut World, actor_id: &str, line: &str) -> usize {
    let player = player_client(world, actor_id);
    let mark = player.seat.mark();
    let (word, args) = parse_typed(line);
    player.seat.send(word, args);
    mark
}

pub fn stop_host(world: &mut World) {
    let live = live_mut(world);
    live.players.clear();
    live.ws_gm.take();
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

/// Releases the claim and waits until the table has let go of it.
pub fn close_actor(world: &mut World, actor_id: &str) {
    let player = player_client(world, actor_id);
    let id = player.seat.send("release", json!({}));
    player.seat.wait_reply(&id);
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
    let text = gm_call(world, 91, "release", "{}");
    assert!(
        text.contains("{\"kind\":\"released\"}"),
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
        let mcp_addr = mcp_addr(world).parse().unwrap();
        let gm = support::block_on(GmClient::connect(mcp_addr))
            .unwrap_or_else(|err| panic!("status: {err}"));
        let live = live_mut(world);
        live.gm = Some(gm);
        live.gm_probe = true;
    }
    call_gm_tool(world, "status", "{}")
}

/// One GM tool call; the body of its reply.
pub fn call_gm_tool(world: &mut World, word: &str, arguments: &str) -> Value {
    read_reply(&gm_call(world, 1, word, arguments))
}

/// The body of the reply in a tool call's result line: the last message.
pub fn read_reply(line: &str) -> Value {
    let value: Value =
        serde_json::from_str(line).unwrap_or_else(|err| panic!("result json: {err}\n{line}"));
    let messages = value["result"]["structuredContent"]["messages"].as_array();
    messages
        .and_then(|messages| messages.last())
        .map(|reply| reply["body"].clone())
        .unwrap_or_else(|| panic!("no reply\n{line}"))
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

/// A new player connection claims `actor`. Returns the claim's reply line.
pub fn claim(world: &World, actor: &str) -> String {
    let seat = Seat::open(live(world).game_port, "player");
    let id = seat.send("claim", json!({ "actor": actor }));
    seat.wait_reply(&id)
}

/// Opens a player connection, claims `actor_id`, and waits for the first view.
fn connect_actor(port: u16, actor_id: &str) -> Seat {
    let seat = Seat::open(port, "player");
    let id = seat.send("claim", json!({ "actor": actor_id }));
    let reply = seat.wait_reply(&id);
    assert!(
        reply.contains("\"kind\":\"claimed\""),
        "{actor_id} was not seated\n{reply}"
    );
    wait_suffix(&seat.text, 0, "\"kind\":\"view\"");
    seat
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

pub fn run(world: &mut World, args: &[&str]) {
    let output = support::run_spook(args);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}
