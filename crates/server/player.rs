//! The game port: accept a player's socket, read the hello, then run its lines.

use std::io;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use spookery_engine::Reason;

use super::frames::Refusal;
use super::seat::{seat, write_all_flush, write_frames, Conn, Inbound, Outbound};
use super::table::{report_hangup, HelloReply, Request};

/// Accept players until the listener fails. Each socket is tuned, then served
/// on a task of its own.
pub(super) async fn serve(
    listener: tokio::net::TcpListener,
    tx: Sender<Request>,
    next_conn: Arc<AtomicU64>,
    keepalive_secs: u64,
    hello_timeout: Duration,
) {
    while let Ok((stream, _)) = listener.accept().await {
        if tune(&stream, keepalive_secs).is_err() {
            continue;
        }
        let conn = Conn::mint(&next_conn);
        tokio::spawn(serve_connection(stream, tx.clone(), conn, hello_timeout));
    }
}

fn tune(stream: &tokio::net::TcpStream, keepalive_secs: u64) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(keepalive_secs))
        .with_interval(Duration::from_secs(1))
        .with_retries(3);
    socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)
}

async fn serve_connection(
    mut stream: tokio::net::TcpStream,
    tx: Sender<Request>,
    conn: Conn,
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
            Ok(Ok(None)) | Ok(Err(super::limits::ReadLineError::Io)) => {
                report_hangup(&tx, conn).await;
                return;
            }
            Ok(Err(super::limits::ReadLineError::TooLong)) => {
                refuse(&mut stream, &Refusal::from(Reason::LineTooLong)).await;
                return;
            }
            Ok(Err(super::limits::ReadLineError::BadUtf8)) => {
                refuse(&mut stream, &Refusal::from(Reason::Schema)).await;
                return;
            }
        }
    };
    let (out, inbound) = seat();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if tx
        .send(Request::Hello {
            conn,
            line,
            seat: out.clone(),
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
        HelloReply::Refused(refusal) => {
            drop(out);
            drop(inbound);
            refuse(&mut stream, &refusal).await;
        }
        HelloReply::Seated => {
            // The queue is the bound. Ask for a small send buffer so a client
            // that stops reading fills the queue. The OS may grant more, and
            // the queue's byte cap still drops that seat.
            let _ = socket2::SockRef::from(&stream)
                .set_send_buffer_size(super::limits::REQUESTED_SEND_BUFFER);
            serve_lines(stream, tx, conn, out, inbound).await;
        }
    }
}

async fn serve_lines(
    stream: tokio::net::TcpStream,
    tx: Sender<Request>,
    conn: Conn,
    out: Outbound,
    inbound: Inbound,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader =
        tokio::io::BufReader::with_capacity(super::limits::PLAYER_LINE_CAP + 1, read_half);
    let mut cancel_rx = inbound.cancel.clone();
    // The sink owns the write half, so it shuts down when `write_frames` returns.
    let writer = tokio::spawn(write_frames(
        async move |frame: &[u8]| write_all_flush(&mut write_half, frame).await,
        inbound,
    ));
    let mut bucket =
        super::limits::TokenBucket::new(super::limits::RATE_PER_SEC, super::limits::BURST);
    loop {
        // Wait before reading so a full window slows the sender. The line is kept.
        if !super::limits::wait_for_token(&mut bucket, &mut cancel_rx).await {
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
                let _ = bucket.try_take(tokio::time::Instant::now());
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                if tx
                    .send(Request::Line {
                        conn,
                        line,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    break;
                }
                // A seat the table dropped is cancelled, so the next wait ends the loop.
                if reply_rx.await.is_err() {
                    break;
                }
            }
            Ok(None) | Err(super::limits::ReadLineError::Io) => {
                report_hangup(&tx, conn).await;
                break;
            }
            Err(super::limits::ReadLineError::TooLong) => {
                let line = Refusal::from(Reason::LineTooLong).line();
                out.send_last(line.into_bytes()).await;
                report_hangup(&tx, conn).await;
                break;
            }
            Err(super::limits::ReadLineError::BadUtf8) => {
                let _ = bucket.try_take(tokio::time::Instant::now());
                let line = Refusal::from(Reason::Schema).line();
                if !out.send(line.into_bytes()) {
                    report_hangup(&tx, conn).await;
                    break;
                }
            }
        }
    }
    drop(out);
    let _ = writer.await;
}

/// How long a refused socket waits for its client to hang up.
const REFUSE_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Most bytes a refused socket reads before it closes anyway.
const REFUSE_DRAIN_BYTES: usize = 64 * 1024;

/// Sends the refusal so it always arrives, then closes.
///
/// A client may pipeline bytes after its hello. Closing a socket that still
/// holds unread bytes sends RST on
/// Windows, and the client's stack then drops the refusal it already got. So
/// the seat sends FIN after the refusal and reads until the client hangs up,
/// for at most `REFUSE_DRAIN_TIMEOUT` and `REFUSE_DRAIN_BYTES`.
async fn refuse(stream: &mut tokio::net::TcpStream, refusal: &Refusal) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if stream.write_all(refusal.line().as_bytes()).await.is_err()
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
    let _ = tokio::time::timeout(REFUSE_DRAIN_TIMEOUT, drain).await;
}
