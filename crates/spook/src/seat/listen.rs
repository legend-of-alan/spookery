use std::io;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use spookery_kernel::shard::open_shard;
use spookery_kernel::Reason;

use super::gm;
use super::http;
use super::thread::{self, Ack, HelloReply, Msg, PlayerReply, SocketEnd};

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
    let mut shard = open_shard(pack).map_err(Error::Reason)?;
    if thread::clear_crashed(&mut shard).map_err(Error::Reason)? {
        thread::print_host(&shard);
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
    thread::serve(shard, rx, &restart_tx)
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
            refuse(&mut stream, line.as_bytes()).await;
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
            let _ = writer.await;
        }
        HelloReply::Gm => {
            let (read_half, write_half) = stream.into_split();
            gm::serve(read_half, write_half, out_tx, out_rx, tx, conn).await;
        }
    }
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

async fn write_frame(write: &mut tokio::net::tcp::OwnedWriteHalf, frame: &[u8]) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    write.write_all(frame).await?;
    write.flush().await?;
    Ok(())
}

async fn read_lf_line<R: tokio::io::AsyncRead + Unpin>(
    stream: &mut R,
) -> io::Result<Option<String>> {
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
