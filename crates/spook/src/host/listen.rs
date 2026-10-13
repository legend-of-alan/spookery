use std::io;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use spookery_kernel::world::World;
use spookery_kernel::Reason;

use super::gm_seat;
use super::http;
use super::world_loop::{self, Ack, HelloReply, Msg, PlayerReply, SocketEnd};

pub enum Error {
    Reason(Reason),
    Io(io::Error),
}

pub fn listen(pack: &Path, addr: SocketAddr, mcp_http: Option<SocketAddr>) -> Result<(), Error> {
    let http_listener = match mcp_http {
        Some(http_addr) => {
            let listener = TcpListener::bind(http_addr).map_err(Error::Io)?;
            listener.set_nonblocking(true).map_err(Error::Io)?;
            Some(listener)
        }
        None => None,
    };
    let listener = TcpListener::bind(addr).map_err(Error::Io)?;
    let mut shard = World::open(pack).map_err(Error::Reason)?;
    if world_loop::clear_crashed(&mut shard).map_err(Error::Reason)? {
        world_loop::print_host(&shard);
    }
    // Keepalive stays the pack interval. The idle switch does not change it.
    let keepalive_secs = shard.budget().tick_every_real_seconds;
    let arm_idle_timer = shard.budget().idle_ticking;
    let (tx, rx) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(Error::Io)?;
    let handle = runtime.handle().clone();
    let started = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&started);
    let tx_accept = tx.clone();
    let accept_handle = handle.clone();
    let next_conn = Arc::new(AtomicU64::new(1));
    let ids = Arc::clone(&next_conn);
    std::thread::spawn(move || {
        flag.store(true, Ordering::Release);
        while let Ok((stream, _)) = listener.accept() {
            if tune(&stream, keepalive_secs).is_err() || stream.set_nonblocking(true).is_err() {
                continue;
            }
            let id = ids.fetch_add(1, Ordering::Relaxed);
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
    if let Some(std_listener) = http_listener {
        let listener = {
            let _entered = handle.enter();
            tokio::net::TcpListener::from_std(std_listener).map_err(Error::Io)?
        };
        let tx_http = tx.clone();
        let ids = Arc::clone(&next_conn);
        handle.spawn(async move {
            http::serve(listener, tx_http, ids).await;
        });
    }
    let (restart_tx, restart_rx) = tokio::sync::watch::channel(0u64);
    if arm_idle_timer {
        let tx_timer = tx.clone();
        handle.spawn(async move {
            let mut restart_rx = restart_rx;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(keepalive_secs)) => {
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
    } else {
        drop(restart_rx);
    }
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
    world_loop::run_world_loop(shard, rx, &restart_tx)
}

fn tune(stream: &std::net::TcpStream, keepalive_secs: u64) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(keepalive_secs))
        .with_interval(Duration::from_secs(1))
        .with_retries(3);
    socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)
}

async fn connection(mut stream: tokio::net::TcpStream, tx: Sender<Msg>, conn: u64) {
    let line = {
        let read = async {
            let mut reader = super::limits::Unbuffered::new(&mut stream);
            super::limits::read_capped_line(&mut reader, super::limits::PLAYER_LINE_CAP).await
        };
        match tokio::time::timeout(super::limits::hello_timeout(), read).await {
            Err(_) => return,
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => {
                report_fault(&tx, conn, SocketEnd::Eof).await;
                return;
            }
            Ok(Err(super::limits::LineFail::Io)) => {
                report_fault(&tx, conn, SocketEnd::Read).await;
                return;
            }
            Ok(Err(super::limits::LineFail::TooLong)) => {
                refuse(&mut stream, &refused_frame(Reason::LineTooLong)).await;
                return;
            }
            Ok(Err(super::limits::LineFail::BadUtf8)) => {
                refuse(&mut stream, &refused_frame(Reason::Schema)).await;
                return;
            }
        }
    };
    let (out_tx, out_rx, cancel_tx, cancel_rx) = super::limits::outbound();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if tx
        .send(Msg::Hello {
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
    let reply = match reply_rx.await {
        Ok(reply) => reply,
        Err(_) => return,
    };
    match reply {
        HelloReply::Refused(line) => {
            drop(out_tx);
            drop(out_rx);
            drop(cancel_rx);
            refuse(&mut stream, line.as_bytes()).await;
        }
        HelloReply::Player => {
            // The queue is the bound. A small send buffer makes a client that
            // stops reading fill that queue instead of the kernel's window.
            let _ = socket2::SockRef::from(&stream).set_send_buffer_size(8 * 1024);
            player_session(stream, tx, conn, out_tx, out_rx, cancel_rx).await;
        }
        HelloReply::Gm => {
            let (read_half, write_half) = stream.into_split();
            gm_seat::serve(read_half, write_half, out_tx, out_rx, cancel_rx, tx, conn).await;
        }
    }
}

async fn player_session(
    stream: tokio::net::TcpStream,
    tx: Sender<Msg>,
    conn: u64,
    out_tx: super::limits::OutTx,
    out_rx: tokio::sync::mpsc::Receiver<super::limits::Outbound>,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
) {
    let (read_half, write_half) = stream.into_split();
    let mut reader =
        tokio::io::BufReader::with_capacity(super::limits::PLAYER_LINE_CAP + 1, read_half);
    let writer = tokio::spawn(super::limits::write_frames(
        write_half,
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
                    .send(Msg::Player {
                        conn,
                        line,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    break;
                }
                match reply_rx.await {
                    Ok(PlayerReply::Done) => {}
                    Ok(PlayerReply::Hangup) | Err(_) => break,
                }
            }
            Ok(None) => {
                report_fault(&tx, conn, SocketEnd::Eof).await;
                break;
            }
            Err(super::limits::LineFail::TooLong) => {
                super::limits::send_last(&out_tx, refused_frame(Reason::LineTooLong)).await;
                report_fault(&tx, conn, SocketEnd::Close).await;
                break;
            }
            Err(super::limits::LineFail::BadUtf8) => {
                let _ = bucket.allow(std::time::Instant::now());
                if !super::limits::try_frame(&out_tx, refused_frame(Reason::Schema)) {
                    report_fault(&tx, conn, SocketEnd::Write).await;
                    break;
                }
            }
            Err(super::limits::LineFail::Io) => {
                report_fault(&tx, conn, SocketEnd::Read).await;
                break;
            }
        }
    }
    drop(out_tx);
    let _ = writer.await;
}

fn refused_frame(code: Reason) -> Vec<u8> {
    super::frames::player_refused_line(code.token()).into_bytes()
}

pub(super) async fn report_fault(tx: &Sender<Msg>, conn: u64, end: SocketEnd) {
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    if tx
        .send(Msg::Fault {
            conn,
            end,
            ack: Ack::Task(ack_tx),
        })
        .is_err()
    {
        return;
    }
    let _ = ack_rx.await;
}

async fn write_bytes(stream: &mut tokio::net::TcpStream, bytes: &[u8]) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    stream.write_all(bytes).await?;
    stream.flush().await?;
    Ok(())
}

/// How long a refused socket waits for its client to hang up.
const REFUSE_DRAIN: Duration = Duration::from_secs(2);

/// Most bytes a refused socket reads before it closes anyway.
const REFUSE_DRAIN_BYTES: usize = 64 * 1024;

/// Sends the refusal so it always arrives, then closes.
///
/// A client may pipeline lines after its hello, as the GM hello does with
/// `initialize`. Closing a socket that still holds unread bytes sends RST on
/// Windows, and the client's stack then drops the refusal it already got. So
/// the seat sends FIN after the refusal and reads until the client hangs up,
/// for at most `REFUSE_DRAIN` and `REFUSE_DRAIN_BYTES`.
async fn refuse(stream: &mut tokio::net::TcpStream, line: &[u8]) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if write_bytes(stream, line).await.is_err() {
        return;
    }
    if stream.shutdown().await.is_err() {
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
