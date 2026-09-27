use std::io::{self, Write};
use std::net::TcpListener;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParam, CallToolResult, JsonObject, ListToolsResult, PaginatedRequestParam, ServerInfo,
    Tool, ToolsCapability,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use spookery_kernel::model::{ActorLive, Presence};
use spookery_kernel::perceive::{perceive, ClockView, KnownFact, Perceive, StockView};
use spookery_kernel::store::{open_shard, Shard};
use spookery_kernel::{idle_digest, Code, DigestViewer};
use spookery_tick::commit_presence;

mod round;
mod speech;

pub use round::{commit_queued, on_scheduled_wake, queue_wait};
pub use speech::{player_scrollback, speak};

const GM_TOOLS: &[&str] = &[
    "session_open",
    "session_status",
    "perceive",
    "intent",
    "say",
    "resolve_check",
    "apply_effect",
    "npc_act",
    "commit_turn",
    "recall",
    "session_save",
];

pub enum Fail {
    Code(Code),
    Io(io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketEnd {
    Eof,
    Read,
    Write,
    Keepalive,
    Close,
}

pub fn listen(pack: &Path) -> Result<(), Fail> {
    let listener = TcpListener::bind("127.0.0.1:9737").map_err(Fail::Io)?;
    let mut shard = open_shard(pack).map_err(Fail::Code)?;
    if clear_crashed(&mut shard).map_err(Fail::Code)? {
        print_host(&shard);
    }
    let idle = shard.budget().tick_every_real_seconds;
    let (tx, rx) = mpsc::channel();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(Fail::Io)?;
    let handle = runtime.handle().clone();
    let started = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&started);
    let tx_accept = tx.clone();
    let accept_handle = handle.clone();
    std::thread::spawn(move || {
        flag.store(true, Ordering::Release);
        while let Ok((stream, _)) = listener.accept() {
            if tune(&stream, idle).is_err() || stream.set_nonblocking(true).is_err() {
                continue;
            }
            let id = next_conn();
            let tx_conn = tx_accept.clone();
            let stream = {
                let _entered = accept_handle.enter();
                match tokio::net::TcpStream::from_std(stream) {
                    Ok(stream) => stream,
                    Err(_) => continue,
                }
            };
            accept_handle.spawn(async move {
                connection(stream, tx_conn, id).await;
            });
        }
    });
    while !started.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    let tx_timer = tx.clone();
    let (restart_tx, restart_rx) = tokio::sync::watch::channel(0u64);
    handle.spawn(async move {
        let mut restart_rx = restart_rx;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(idle)) => {
                    if tx_timer.send(Msg::Wake).is_err() {
                        break;
                    }
                }
                changed = restart_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    });
    let tx_stdin = tx.clone();
    std::thread::spawn(move || {
        for line in io::stdin().lines() {
            match line {
                Ok(line) => {
                    if tx_stdin.send(Msg::Stdin(line)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx_stdin.send(Msg::StdinEof);
    });
    let mut seats = Seats::default();
    loop {
        let msg = match rx.recv() {
            Ok(msg) => msg,
            Err(_) => {
                eprintln!("{}", Code::NotInSpec);
                std::process::exit(1);
            }
        };
        if let Err(code) = on_msg(&mut shard, &mut seats, &restart_tx, msg) {
            eprintln!("{code}");
            std::process::exit(1);
        }
    }
}

pub fn seat_player(shard: &mut Shard, actor_id: &str) -> Result<String, Code> {
    if !shard.now().actors.get(actor_id).is_some_and(ActorLive::is_pc) {
        return Err(Code::Schema);
    }
    let session_id = mint_session_id();
    let mut presence = shard.now().presence.clone();
    presence.session_ids.push(session_id.clone());
    if !presence.actors_present.iter().any(|id| id == actor_id) {
        presence.actors_present.push(actor_id.to_string());
    }
    presence.humans = u32::try_from(presence.session_ids.len()).map_err(|_| Code::Schema)?;
    commit_presence(shard, presence)?;
    Ok(session_id)
}

pub fn release_player_seat(
    shard: &mut Shard,
    session_id: &str,
    actor_id: &str,
    end: SocketEnd,
) -> Result<bool, Code> {
    match end {
        SocketEnd::Eof | SocketEnd::Read | SocketEnd::Write | SocketEnd::Keepalive | SocketEnd::Close => {}
    }
    let mut presence = shard.now().presence.clone();
    presence.session_ids.retain(|id| id != session_id);
    presence.actors_present.retain(|id| id != actor_id);
    presence.humans = u32::try_from(presence.session_ids.len()).map_err(|_| Code::Schema)?;
    commit_presence(shard, presence)
}

fn clear_crashed(shard: &mut Shard) -> Result<bool, Code> {
    let presence = &shard.now().presence;
    if presence.session_ids.is_empty() && presence.actors_present.is_empty() && presence.humans == 0 {
        return Ok(false);
    }
    commit_presence(
        shard,
        Presence {
            humans: 0,
            session_ids: Vec::new(),
            actors_present: Vec::new(),
        },
    )
}

fn mint_session_id() -> String {
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("s{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn next_conn() -> u64 {
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn tune(stream: &std::net::TcpStream, idle_secs: u64) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(idle_secs))
        .with_interval(Duration::from_secs(1))
        .with_retries(3);
    socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)
}

fn print_host(shard: &Shard) {
    if let Ok(text) = idle_digest(shard, DigestViewer::Host) {
        if !text.is_empty() {
            print!("{text}");
            let _ = io::stdout().flush();
        }
    }
}

struct Seats {
    player: Option<HeldPlayer>,
    gm: Option<GmSeat>,
}

impl Default for Seats {
    fn default() -> Self {
        Self { player: None, gm: None }
    }
}

struct HeldPlayer {
    conn: u64,
    actor_id: String,
    session_id: String,
    out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

struct GmSeat {
    conn: u64,
    out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

enum Msg {
    Hello {
        conn: u64,
        line: String,
        out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
        reply: tokio::sync::oneshot::Sender<HelloReply>,
    },
    Player {
        conn: u64,
        line: String,
        reply: tokio::sync::oneshot::Sender<PlayerReply>,
    },
    Fault {
        conn: u64,
        end: SocketEnd,
        ack: tokio::sync::oneshot::Sender<()>,
    },
    Tool {
        name: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
        reply: tokio::sync::oneshot::Sender<serde_json::Value>,
    },
    Wake,
    Stdin(String),
    StdinEof,
}

enum HelloReply {
    Refused(String),
    Player,
    Gm,
}

enum PlayerReply {
    Done,
    Hangup,
}

fn on_msg(
    shard: &mut Shard,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    msg: Msg,
) -> Result<(), Code> {
    match msg {
        Msg::Hello { conn, line, out, reply } => {
            let answer = on_hello(shard, seats, conn, &line, out)?;
            let _ = reply.send(answer);
        }
        Msg::Player { conn, line, reply } => {
            let answer = on_player(shard, seats, restart, conn, &line)?;
            let _ = reply.send(answer);
        }
        Msg::Fault { conn, end, ack } => on_fault(shard, seats, conn, end, ack)?,
        Msg::Tool { name, arguments, reply } => {
            let done = round::gm_tool(shard, &name, arguments);
            if done.ticked {
                refresh_player(shard, seats);
                print_host(shard);
            }
            if let Some(frame) = done.gm_event {
                if let Some(gm) = &seats.gm {
                    let _ = gm.out.send(frame);
                }
            }
            if done.advanced {
                bump_sleep(restart);
            }
            let _ = reply.send(done.value);
        }
        Msg::Wake => {
            if on_scheduled_wake(shard, seats.player.is_some(), seats.gm.is_some())? {
                bump_sleep(restart);
            }
        }
        Msg::Stdin(line) if line == "stop" => shutdown(shard, seats),
        Msg::Stdin(_) => eprintln!("{}", Code::NotInSpec),
        Msg::StdinEof => shutdown(shard, seats),
    }
    Ok(())
}

fn bump_sleep(restart: &tokio::sync::watch::Sender<u64>) {
    let next = restart.borrow().wrapping_add(1);
    let _ = restart.send(next);
}

fn refresh_player(shard: &Shard, seats: &Seats) {
    let Some(player) = &seats.player else {
        return;
    };
    if let Ok(text) = player_refresh(shard, &player.actor_id) {
        let _ = player.out.send(text.into_bytes());
    }
}

fn shutdown(shard: &mut Shard, seats: &mut Seats) -> ! {
    if let Some(player) = seats.player.take() {
        match release_player_seat(shard, &player.session_id, &player.actor_id, SocketEnd::Eof) {
            Ok(true) => print_host(shard),
            Ok(false) => {}
            Err(code) => {
                eprintln!("{code}");
                std::process::exit(1);
            }
        }
    }
    seats.gm = None;
    std::process::exit(0);
}



fn on_hello(
    shard: &mut Shard,
    seats: &mut Seats,
    conn: u64,
    line: &str,
    out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
) -> Result<HelloReply, Code> {
    if line == "gm" {
        if seats.gm.is_some() {
            return Ok(HelloReply::Refused(refused_line(Code::SessionFull)));
        }
        for frame in speech::gm_events(shard.root())? {
            let _ = out.send(frame);
        }
        seats.gm = Some(GmSeat { conn, out });
        return Ok(HelloReply::Gm);
    }
    if let Some(id) = player_hello(line) {
        if seats.player.is_some() {
            return Ok(HelloReply::Refused(refused_line(Code::SessionFull)));
        }
        let session_id = match seat_player(shard, id) {
            Ok(session_id) => session_id,
            Err(code) => return Ok(HelloReply::Refused(refused_line(code))),
        };
        let bytes = match welcome(shard, id) {
            Ok(bytes) => bytes,
            Err(code) => {
                if release_player_seat(shard, &session_id, id, SocketEnd::Close)? {
                    print_host(shard);
                }
                return Ok(HelloReply::Refused(refused_line(code)));
            }
        };
        let _ = out.send(bytes);
        seats.player = Some(HeldPlayer {
            conn,
            actor_id: id.to_string(),
            session_id,
            out,
        });
        print_host(shard);
        return Ok(HelloReply::Player);
    }
    Ok(HelloReply::Refused(refused_line(Code::Schema)))
}

fn player_hello(line: &str) -> Option<&str> {
    let id = line.strip_prefix("player:")?;
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return None;
    }
    Some(id)
}

fn welcome(shard: &Shard, actor_id: &str) -> Result<Vec<u8>, Code> {
    let mut out = String::new();
    for line in player_scrollback(shard.root(), actor_id)? {
        out.push_str(&line);
    }
    out.push_str(&player_refresh(shard, actor_id)?);
    Ok(out.into_bytes())
}

fn player_refresh(shard: &Shard, actor_id: &str) -> Result<String, Code> {
    let digest = idle_digest(shard, DigestViewer::Player(actor_id.to_string()))?;
    let view = perceive(shard, &format!("player:{actor_id}"))?;
    let mut out = resume_line(&digest);
    out.push_str(&perceive_line(&view));
    Ok(out)
}

fn on_player(
    shard: &mut Shard,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    line: &str,
) -> Result<PlayerReply, Code> {
    if is_close(line) {
        if seats.player.as_ref().is_some_and(|player| player.conn == conn) {
            let player = seats.player.take().expect("player seat");
            if release_player_seat(shard, &player.session_id, &player.actor_id, SocketEnd::Close)? {
                print_host(shard);
            }
        }
        return Ok(PlayerReply::Hangup);
    }
    let Some(actor_id) = seats
        .player
        .as_ref()
        .filter(|player| player.conn == conn)
        .map(|player| player.actor_id.clone())
    else {
        return Ok(PlayerReply::Hangup);
    };
    match speak(shard, &actor_id, line) {
        Ok(spoken) => {
            println!("{}", spoken.tape);
            let _ = io::stdout().flush();
            if let Some(gm) = &seats.gm {
                let _ = gm.out.send(spoken.gm_event);
            }
            send_player(seats, conn, spoken.player_event.into_bytes());
            Ok(PlayerReply::Done)
        }
        Err(Code::NotInSpec) => on_mechanical(shard, seats, restart, conn, &actor_id, line),
        Err(code) => {
            send_player(seats, conn, refused_line(code).into_bytes());
            Ok(PlayerReply::Done)
        }
    }
}

fn on_mechanical(
    shard: &mut Shard,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    actor_id: &str,
    line: &str,
) -> Result<PlayerReply, Code> {
    let gm_seated = seats.gm.is_some();
    match round::mechanical(shard, actor_id, line, gm_seated) {
        Ok(round::Mechanical::Queued(note)) => {
            if let Some(gm) = &seats.gm {
                let _ = gm.out.send(note.gm_frame());
            }
            send_player(seats, conn, note.player_line().into_bytes());
        }
        Ok(round::Mechanical::Committed { advanced }) => {
            if advanced {
                bump_sleep(restart);
            }
            print_host(shard);
            match player_refresh(shard, actor_id) {
                Ok(bytes) => send_player(seats, conn, bytes.into_bytes()),
                Err(code) => send_player(seats, conn, refused_line(code).into_bytes()),
            }
        }
        Err(code) => send_player(seats, conn, refused_line(code).into_bytes()),
    }
    Ok(PlayerReply::Done)
}

fn send_player(seats: &Seats, conn: u64, bytes: Vec<u8>) {
    if let Some(player) = seats.player.as_ref().filter(|player| player.conn == conn) {
        let _ = player.out.send(bytes);
    }
}

fn is_close(line: &str) -> bool {
    match line.split_once(' ') {
        Some((token, rest)) => token == "close" && rest.is_empty(),
        None => line == "close",
    }
}

fn on_fault(
    shard: &mut Shard,
    seats: &mut Seats,
    conn: u64,
    end: SocketEnd,
    ack: tokio::sync::oneshot::Sender<()>,
) -> Result<(), Code> {
    let result = if seats.player.as_ref().is_some_and(|player| player.conn == conn) {
        let player = seats.player.take().expect("player seat");
        match release_player_seat(shard, &player.session_id, &player.actor_id, end) {
            Ok(true) => {
                print_host(shard);
                Ok(())
            }
            Ok(false) => Ok(()),
            Err(code) => Err(code),
        }
    } else if seats.gm.as_ref().is_some_and(|gm| gm.conn == conn) {
        seats.gm = None;
        Ok(())
    } else {
        Ok(())
    };
    let _ = ack.send(());
    result
}

fn refused_line(code: Code) -> String {
    format!("{{\"kind\":\"refused\",\"code\":\"{}\"}}\n", code.token())
}

fn resume_line(text: &str) -> String {
    let mut out = String::from("{\"kind\":\"resume\",\"text\":");
    push_string(&mut out, text);
    out.push_str("}\n");
    out
}

fn perceive_line(view: &Perceive) -> String {
    let mut body = render_perceive(view);
    body.insert_str(1, "\"kind\":\"perceive\",");
    body
}

async fn connection(mut stream: tokio::net::TcpStream, tx: Sender<Msg>, conn: u64) {
    let line = match read_lf_line(&mut stream).await {
        Ok(Some(line)) => line,
        Ok(None) => {
            report_fault(&tx, conn, SocketEnd::Eof).await;
            return;
        }
        Err(_) => {
            report_fault(&tx, conn, SocketEnd::Read).await;
            return;
        }
    };
    let (out_tx, out_rx) = tokio::sync::mpsc::unbounded_channel();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if tx
        .send(Msg::Hello {
            conn,
            line,
            out: out_tx.clone(),
            reply: reply_tx,
        })
        .is_err()
    {
        return;
    }
    let reply = match reply_rx.await {
        Ok(reply) => reply,
        Err(_) => return,
    };
    match reply {
        HelloReply::Refused(line) => {
            drop(out_tx);
            drop(out_rx);
            let _ = write_bytes(&mut stream, line.as_bytes()).await;
        }
        HelloReply::Player => {
            let (mut read_half, write_half) = stream.into_split();
            let writer = tokio::spawn(async move {
                let mut out_rx = out_rx;
                let mut write_half = write_half;
                while let Some(frame) = out_rx.recv().await {
                    if write_frame(&mut write_half, &frame).await.is_err() {
                        break;
                    }
                }
            });
            drop(out_tx);
            loop {
                let line = match read_lf_line(&mut read_half).await {
                    Ok(Some(line)) => line,
                    Ok(None) => {
                        report_fault(&tx, conn, SocketEnd::Eof).await;
                        break;
                    }
                    Err(_) => {
                        report_fault(&tx, conn, SocketEnd::Read).await;
                        break;
                    }
                };
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                if tx.send(Msg::Player { conn, line, reply: reply_tx }).is_err() {
                    break;
                }
                match reply_rx.await {
                    Ok(PlayerReply::Done) => {}
                    Ok(PlayerReply::Hangup) | Err(_) => break,
                }
            }
            let _ = writer.await;
        }
        HelloReply::Gm => {
            let (read_half, write_half) = stream.into_split();
            let writer_task = tokio::spawn(async move {
                let mut out_rx = out_rx;
                let mut write_half = write_half;
                while let Some(frame) = out_rx.recv().await {
                    if write_frame(&mut write_half, &frame).await.is_err() {
                        break;
                    }
                }
            });
            let writer = LineWriter {
                tx: out_tx,
                buf: Vec::new(),
            };
            let host = GmHost { tx: tx.clone() };
            match host.serve((read_half, writer)).await {
                Ok(running) => {
                    let _ = running.waiting().await;
                }
                Err(_) => {}
            }
            report_fault(&tx, conn, SocketEnd::Eof).await;
            let _ = writer_task.await;
        }
    }
}

async fn report_fault(tx: &Sender<Msg>, conn: u64, end: SocketEnd) {
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    if tx.send(Msg::Fault { conn, end, ack: ack_tx }).is_err() {
        return;
    }
    let _ = ack_rx.await;
}

async fn write_frame(write: &mut tokio::net::tcp::OwnedWriteHalf, frame: &[u8]) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    write.write_all(frame).await?;
    write.flush().await?;
    Ok(())
}

struct LineWriter {
    tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    buf: Vec<u8>,
}

impl tokio::io::AsyncWrite for LineWriter {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        let writer = self.get_mut();
        writer.buf.extend_from_slice(data);
        while let Some(index) = writer.buf.iter().position(|byte| *byte == b'\n') {
            let frame: Vec<u8> = writer.buf.drain(..=index).collect();
            if writer.tx.send(frame).is_err() {
                return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "gm writer closed")));
            }
        }
        Poll::Ready(Ok(data.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

async fn read_lf_line<R: tokio::io::AsyncRead + Unpin>(stream: &mut R) -> io::Result<Option<String>> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte).await {
            Ok(0) => {
                if buf.is_empty() {
                    return Ok(None);
                }
                break;
            }
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(err) => return Err(err),
        }
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

async fn write_bytes(stream: &mut tokio::net::TcpStream, bytes: &[u8]) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    stream.write_all(bytes).await?;
    stream.flush().await?;
    Ok(())
}

#[derive(Clone)]
struct GmHost {
    tx: Sender<Msg>,
}

impl ServerHandler for GmHost {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities.tools = Some(ToolsCapability { list_changed: None });
        info
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let schema = std::sync::Arc::new(object_schema());
        let tools = GM_TOOLS
            .iter()
            .map(|name| Tool::new(*name, *name, std::sync::Arc::clone(&schema)))
            .collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let tx = self.tx.clone();
        let name = request.name.into_owned();
        let arguments = request.arguments;
        async move {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            tx.send(Msg::Tool {
                name,
                arguments,
                reply: reply_tx,
            })
            .map_err(|_| McpError::internal_error("shard closed", None))?;
            let value = reply_rx
                .await
                .map_err(|_| McpError::internal_error("shard closed", None))?;
            Ok(CallToolResult::structured(value))
        }
    }
}

fn object_schema() -> JsonObject {
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), "object".into());
    schema
}

fn render_perceive(view: &Perceive) -> String {
    let mut out = String::from("{");
    field_str(&mut out, "viewer", &view.viewer, true);
    field_str(&mut out, "place", &view.place, false);
    out.push_str(",\"visible_actors\":");
    push_strings(&mut out, &view.visible_actors);
    out.push_str(",\"known_facts\":[");
    for (index, fact) in view.known_facts.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_fact(&mut out, fact);
    }
    out.push_str("],\"messages\":[");
    for (index, message) in view.messages.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_message(&mut out, message);
    }
    out.push_str("],\"unknown_mentioned\":");
    push_strings(&mut out, &view.unknown_mentioned);
    out.push_str(",\"clocks\":[");
    for (index, clock) in view.clocks.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_clock(&mut out, clock);
    }
    out.push_str("],\"stocks\":[");
    for (index, stock) in view.stocks.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_stock(&mut out, stock);
    }
    out.push_str("],\"schedule_hint\":null}\n");
    out
}

fn push_fact(out: &mut String, fact: &KnownFact) {
    out.push('{');
    field_str(out, "id", &fact.id, true);
    field_str(out, "text", &fact.text, false);
    field_str(out, "body", &fact.body, false);
    out.push('}');
}

fn push_message(out: &mut String, message: &spookery_kernel::model::Message) {
    out.push('{');
    field_str(out, "id", &message.id, true);
    out.push_str(",\"to\":");
    push_strings(out, &message.to);
    if let Some(place) = &message.place {
        field_str(out, "place", place, false);
    }
    out.push_str(",\"fact_ids\":");
    push_strings(out, &message.fact_ids);
    field_str(out, "text", &message.text, false);
    out.push('}');
}

fn push_clock(out: &mut String, clock: &ClockView) {
    out.push('{');
    field_str(out, "id", &clock.id, true);
    field_i64(out, "value", clock.value, false);
    field_i64(out, "max", clock.max, false);
    field_bool(out, "paused", clock.paused, false);
    out.push('}');
}

fn push_stock(out: &mut String, stock: &StockView) {
    out.push('{');
    field_str(out, "sku", &stock.sku, true);
    field_i64(out, "qty", stock.qty, false);
    out.push('}');
}

fn push_strings(out: &mut String, values: &[String]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_string(out, value);
    }
    out.push(']');
}

fn field_str(out: &mut String, key: &str, value: &str, first: bool) {
    if !first {
        out.push(',');
    }
    push_string(out, key);
    out.push(':');
    push_string(out, value);
}

fn field_i64(out: &mut String, key: &str, value: i64, first: bool) {
    if !first {
        out.push(',');
    }
    push_string(out, key);
    out.push(':');
    out.push_str(&value.to_string());
}

fn field_bool(out: &mut String, key: &str, value: bool, first: bool) {
    if !first {
        out.push(',');
    }
    push_string(out, key);
    out.push(':');
    out.push_str(if value { "true" } else { "false" });
}

fn push_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(other))),
            other => out.push(other),
        }
    }
    out.push('"');
}
