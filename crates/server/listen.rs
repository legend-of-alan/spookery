//! Bind the game port and the agent door.

use std::future::Future;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use spookery_engine::Reason;

use super::mcp_http;
use super::world_loop::{self, HelloReply, WorldMsg};

#[derive(Debug)]
pub enum Error {
    Reason(Reason),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reason(code) => write!(f, "{code}"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

pub struct Config {
    pub addr: SocketAddr,
    pub door: Option<SocketAddr>,
    pub hello_timeout: Duration,
    pub out: Box<dyn Write + Send>,
    pub err: Box<dyn Write + Send>,
}

impl Config {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            door: None,
            hello_timeout: super::limits::HELLO_LIMIT,
            out: Box::new(io::sink()),
            err: Box::new(io::sink()),
        }
    }
}

/// Bind both ports, report them and serve on the caller's runtime until
/// `shutdown` resolves. `world` owns the pack lock until the world loop ends.
/// Returns the world's result.
pub async fn serve(
    world: spookery_engine::World,
    mut config: Config,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error> {
    let listener = bind(config.addr).await?;
    let http_listener = bind(door_request(config.addr, config.door)?).await?;
    let game = listener.local_addr().map_err(Error::Io)?;
    let door = http_listener.local_addr().map_err(Error::Io)?;
    report_bound(&mut *config.out, game, door)?;
    // Keepalive stays the pack interval. The idle switch does not change it.
    let keepalive_secs = world.budget().round_every_real_seconds;
    let arm_idle_timer = world.budget().idle_ticking;
    let hello_timeout = config.hello_timeout;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut tasks = tokio::task::JoinSet::new();
    let next_conn = Arc::new(AtomicU64::new(1));
    let tx_accept = tx.clone();
    let ids = Arc::clone(&next_conn);
    tasks.spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            if tune(&stream, keepalive_secs).is_err() {
                continue;
            }
            let id = ids.fetch_add(1, Ordering::Relaxed);
            let tx_conn = tx_accept.clone();
            tokio::spawn(connection(stream, tx_conn, id, hello_timeout));
        }
    });
    tasks.spawn(mcp_http::serve(http_listener, tx.clone(), next_conn));
    let (restart_tx, mut restart_rx) = tokio::sync::watch::channel(0u64);
    if arm_idle_timer {
        let tx_timer = tx.clone();
        tasks.spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(keepalive_secs)) => {
                        if tx_timer.send(WorldMsg::Wake).is_err() {
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
    }
    let mut world_loop = tokio::task::spawn_blocking(move || {
        world_loop::run_world_loop(world, rx, &restart_tx, &mut *config.out, &mut *config.err)
    });
    // A world loop that ends on its own (a store fault) ends `serve` too.
    let done = tokio::select! {
        done = &mut world_loop => done,
        () = shutdown => {
            let _ = tx.send(WorldMsg::Stop);
            world_loop.await
        }
    };
    done.unwrap_or(Err(Error::Reason(Reason::NotInSpec)))
}

async fn bind(addr: SocketAddr) -> Result<tokio::net::TcpListener, Error> {
    tokio::net::TcpListener::bind(addr).await.map_err(Error::Io)
}

/// The agent door to bind. A fixed game port keeps the next port. Port 0, on
/// the game address or on `--mcp-http`, asks the OS for a port of its own.
pub fn door_request(game: SocketAddr, mcp_addr: Option<SocketAddr>) -> Result<SocketAddr, Error> {
    if let Some(door) = mcp_addr {
        return Ok(door);
    }
    if game.port() == 0 {
        return Ok(SocketAddr::new(game.ip(), 0));
    }
    spookery_protocol::default_door(game).ok_or(Error::Reason(Reason::NotInSpec))
}

fn report_bound(out: &mut dyn Write, game: SocketAddr, door: SocketAddr) -> Result<(), Error> {
    let line = serde_json::json!({
        "kind": "listening",
        "game": game.to_string(),
        "door": door.to_string(),
    });
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(Error::Io)
}

fn tune(stream: &tokio::net::TcpStream, keepalive_secs: u64) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(keepalive_secs))
        .with_interval(Duration::from_secs(1))
        .with_retries(3);
    socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)
}

async fn connection(
    mut stream: tokio::net::TcpStream,
    tx: Sender<WorldMsg>,
    conn: u64,
    hello_timeout: Duration,
) {
    let line = {
        let read = async {
            let mut reader = super::limits::ByteReader::new(&mut stream);
            super::limits::read_capped_line(&mut reader, super::limits::PLAYER_LINE_CAP).await
        };
        match tokio::time::timeout(hello_timeout, read).await {
            Err(_) => return,
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) | Ok(Err(super::limits::LineReadError::Io)) => {
                report_fault(&tx, conn).await;
                return;
            }
            Ok(Err(super::limits::LineReadError::TooLong)) => {
                refuse(&mut stream, &refused_frame(Reason::LineTooLong)).await;
                return;
            }
            Ok(Err(super::limits::LineReadError::BadUtf8)) => {
                refuse(&mut stream, &refused_frame(Reason::Schema)).await;
                return;
            }
        }
    };
    let (out_tx, out_rx, cancel_tx, cancel_rx) = super::limits::outbound();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if tx
        .send(WorldMsg::Hello {
            conn,
            line,
            out: out_tx.clone(),
            cancel: cancel_tx,
            reply: reply_tx,
        })
        .is_err()
    {
        return;
    }
    let Ok(reply) = reply_rx.await else {
        return;
    };
    match reply {
        HelloReply::Refused(line) => {
            drop(out_tx);
            drop(out_rx);
            drop(cancel_rx);
            refuse(&mut stream, line.as_bytes()).await;
        }
        HelloReply::Player => {
            // The queue is the bound. Ask for a small send buffer so a client
            // that stops reading fills the queue. The OS may grant more, and
            // the queue's byte cap still drops that seat.
            let _ = socket2::SockRef::from(&stream)
                .set_send_buffer_size(super::limits::REQUESTED_SEND_BUFFER);
            player_session(stream, tx, conn, out_tx, out_rx, cancel_rx).await;
        }
    }
}

async fn player_session(
    stream: tokio::net::TcpStream,
    tx: Sender<WorldMsg>,
    conn: u64,
    out_tx: super::limits::OutTx,
    out_rx: super::limits::OutRx,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader =
        tokio::io::BufReader::with_capacity(super::limits::PLAYER_LINE_CAP + 1, read_half);
    // The sink owns the write half, so it shuts down when `write_frames` returns.
    let writer = tokio::spawn(super::limits::write_frames(
        async move |frame: &[u8]| super::limits::write_all_flush(&mut write_half, frame).await,
        out_rx,
        cancel_rx.clone(),
    ));
    let mut bucket =
        super::limits::TokenBucket::new(super::limits::RATE_PER_SEC, super::limits::BURST);
    loop {
        // Wait before reading so a full window slows the sender. The line is kept.
        if !super::limits::wait_turn(&mut bucket, &mut cancel_rx).await {
            break;
        }
        let read = super::limits::read_capped_line(&mut reader, super::limits::PLAYER_LINE_CAP);
        tokio::pin!(read);
        let line = tokio::select! {
            biased;
            changed = cancel_rx.changed() => {
                if changed.is_err() || *cancel_rx.borrow() {
                    break;
                }
                continue;
            }
            line = &mut read => line,
        };
        match line {
            Ok(Some(line)) => {
                let _ = bucket.allow(std::time::Instant::now());
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                if tx
                    .send(WorldMsg::Player {
                        conn,
                        line,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    break;
                }
                // A seat the world loop dropped is cancelled, so the next wait ends the loop.
                if reply_rx.await.is_err() {
                    break;
                }
            }
            Ok(None) | Err(super::limits::LineReadError::Io) => {
                report_fault(&tx, conn).await;
                break;
            }
            Err(super::limits::LineReadError::TooLong) => {
                super::limits::send_last(&out_tx, refused_frame(Reason::LineTooLong)).await;
                report_fault(&tx, conn).await;
                break;
            }
            Err(super::limits::LineReadError::BadUtf8) => {
                let _ = bucket.allow(std::time::Instant::now());
                if !super::limits::try_frame(&out_tx, refused_frame(Reason::Schema)) {
                    report_fault(&tx, conn).await;
                    break;
                }
            }
        }
    }
    drop(out_tx);
    let _ = writer.await;
}

fn refused_frame(code: Reason) -> Vec<u8> {
    super::frames::player_refused(code.token(), None).into_bytes()
}

pub(super) async fn report_fault(tx: &Sender<WorldMsg>, conn: u64) {
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    if tx.send(WorldMsg::Fault { conn, ack: ack_tx }).is_err() {
        return;
    }
    let _ = ack_rx.await;
}

/// How long a refused socket waits for its client to hang up.
const REFUSE_DRAIN: Duration = Duration::from_secs(2);

/// Most bytes a refused socket reads before it closes anyway.
const REFUSE_DRAIN_BYTES: usize = 64 * 1024;

/// Sends the refusal so it always arrives, then closes.
///
/// A client may pipeline bytes after its hello. Closing a socket that still
/// holds unread bytes sends RST on
/// Windows, and the client's stack then drops the refusal it already got. So
/// the seat sends FIN after the refusal and reads until the client hangs up,
/// for at most `REFUSE_DRAIN` and `REFUSE_DRAIN_BYTES`.
async fn refuse(stream: &mut tokio::net::TcpStream, line: &[u8]) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if stream.write_all(line).await.is_err()
        || stream.flush().await.is_err()
        || stream.shutdown().await.is_err()
    {
        return;
    }
    let drain = async {
        let mut buf = [0u8; 4096];
        let mut seen = 0usize;
        while seen < REFUSE_DRAIN_BYTES {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => seen = seen.saturating_add(n),
            }
        }
    };
    let _ = tokio::time::timeout(REFUSE_DRAIN, drain).await;
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::door_request;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    #[test]
    fn a_fixed_game_port_uses_the_next_port() {
        assert_eq!(door_request(addr(9737), None).ok(), Some(addr(9738)));
    }

    #[test]
    fn port_zero_asks_for_a_door_of_its_own() {
        assert_eq!(door_request(addr(0), None).ok(), Some(addr(0)));
    }

    #[test]
    fn an_explicit_door_wins() {
        assert_eq!(door_request(addr(0), Some(addr(9))).ok(), Some(addr(9)));
        assert_eq!(door_request(addr(9737), Some(addr(0))).ok(), Some(addr(0)));
    }

    #[test]
    fn the_last_port_has_no_next_port() {
        assert!(door_request(addr(65535), None).is_err());
    }
}
