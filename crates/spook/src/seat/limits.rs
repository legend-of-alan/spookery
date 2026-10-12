//! Caps, the hello deadline, and the per-player token bucket.
//!
//! A hello is read one byte at a time so the GM path can take the socket
//! exactly after the newline. The GM client writes `initialize` on that same
//! write. A buffered hello would keep those bytes. Player lines after the
//! hello use a `BufReader`. The GM TCP door goes through rmcp's combined
//! reader, which calls `read_until` with no cap, so that read half is wrapped.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufRead, AsyncRead, AsyncWrite, ReadBuf};

/// Hello and player lines, not counting the newline.
pub(super) const PLAYER_LINE_CAP: usize = 8 * 1024;

/// GM TCP lines after the hello. Same size as the HTTP body cap.
pub(super) const GM_LINE_CAP: usize = 1024 * 1024;

/// Player lines accepted per second after the burst is spent.
pub(super) const RATE_PER_SEC: u64 = 5;

/// Player lines accepted immediately on a fresh connection.
pub(super) const BURST: u64 = 10;

/// Frames queued toward one socket before that socket is dropped.
pub(super) const OUTBOUND_FRAMES: usize = 256;

const HELLO_LIMIT: Duration = Duration::from_secs(10);

/// Millitokens per line. Refill uses this scale so the bucket stays integer.
const SCALE: u128 = 1000;

pub(super) type OutTx = tokio::sync::mpsc::Sender<Vec<u8>>;
pub(super) type CancelTx = tokio::sync::watch::Sender<bool>;

pub(super) fn outbound() -> (
    OutTx,
    tokio::sync::mpsc::Receiver<Vec<u8>>,
    CancelTx,
    tokio::sync::watch::Receiver<bool>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(OUTBOUND_FRAMES);
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    (tx, rx, cancel_tx, cancel_rx)
}

/// Tests set `SPOOK_HELLO_TIMEOUT_MS`. Any other value is the 10 second limit.
pub(super) fn hello_timeout() -> Duration {
    match std::env::var("SPOOK_HELLO_TIMEOUT_MS") {
        Ok(text) => match text.parse::<u64>() {
            Ok(ms) if ms > 0 => Duration::from_millis(ms),
            _ => HELLO_LIMIT,
        },
        Err(_) => HELLO_LIMIT,
    }
}

#[derive(Debug)]
pub(super) enum LineFail {
    Io,
    TooLong,
    BadUtf8,
}

/// One LF line. `cap` counts every byte before the LF, including a CR.
/// The buffer never holds more than `cap` bytes.
pub(super) async fn read_capped_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    cap: usize,
) -> Result<Option<String>, LineFail> {
    use tokio::io::AsyncBufReadExt;
    let mut buf = Vec::new();
    loop {
        if buf.len() > cap {
            return Err(LineFail::TooLong);
        }
        let room = cap - buf.len();
        let (take, finished, too_long, eof) = {
            let filled = reader.fill_buf().await.map_err(|_| LineFail::Io)?;
            if filled.is_empty() {
                (0, true, false, true)
            } else if let Some(pos) = filled.iter().position(|byte| *byte == b'\n') {
                if pos > room {
                    (room, true, true, false)
                } else {
                    (pos + 1, true, false, false)
                }
            } else if filled.len() > room {
                (room, true, true, false)
            } else {
                (filled.len(), false, false, false)
            }
        };
        if eof {
            if buf.is_empty() {
                return Ok(None);
            }
            return finish(buf);
        }
        if take > 0 {
            let filled = reader.fill_buf().await.map_err(|_| LineFail::Io)?;
            buf.extend_from_slice(&filled[..take]);
            reader.consume(take);
        }
        if too_long {
            return Err(LineFail::TooLong);
        }
        if finished {
            if buf.last() == Some(&b'\n') {
                buf.pop();
            }
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return finish(buf);
        }
    }
}

fn finish(buf: Vec<u8>) -> Result<Option<String>, LineFail> {
    match String::from_utf8(buf) {
        Ok(text) => Ok(Some(text)),
        Err(_) => Err(LineFail::BadUtf8),
    }
}

/// Reads one byte at a time and only the bytes the line reader consumes.
pub(super) struct Unbuffered<R> {
    inner: R,
    buf: [u8; 1],
    len: usize,
}

impl<R> Unbuffered<R> {
    pub(super) fn new(inner: R) -> Self {
        Self {
            inner,
            buf: [0; 1],
            len: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for Unbuffered<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if this.len == 0 {
            let mut tmp = [0u8; 1];
            let mut read = ReadBuf::new(&mut tmp);
            match Pin::new(&mut this.inner).poll_read(cx, &mut read) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Ready(Ok(())) => {
                    if read.filled().is_empty() {
                        return Poll::Ready(Ok(()));
                    }
                    this.buf[0] = read.filled()[0];
                    this.len = 1;
                }
            }
        }
        let n = this.len.min(buf.remaining());
        buf.initialize_unfilled_to(n)
            .copy_from_slice(&this.buf[..n]);
        buf.advance(n);
        if n >= this.len {
            this.len = 0;
        } else {
            this.buf.copy_within(n..this.len, 0);
            this.len -= n;
        }
        Poll::Ready(Ok(()))
    }
}

impl<R: AsyncRead + Unpin> AsyncBufRead for Unbuffered<R> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let this = self.get_mut();
        if this.len > 0 {
            let len = this.len;
            return Poll::Ready(Ok(&this.buf[..len]));
        }
        let mut tmp = [0u8; 1];
        let mut read = ReadBuf::new(&mut tmp);
        match Pin::new(&mut this.inner).poll_read(cx, &mut read) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Ready(Ok(())) => {
                if read.filled().is_empty() {
                    Poll::Ready(Ok(&[]))
                } else {
                    this.buf[0] = read.filled()[0];
                    this.len = 1;
                    Poll::Ready(Ok(&this.buf[..1]))
                }
            }
        }
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        let this = self.get_mut();
        if amt >= this.len {
            this.len = 0;
        } else if amt > 0 {
            this.buf.copy_within(amt..this.len, 0);
            this.len -= amt;
        }
    }
}

/// Stops an `AsyncRead` once a line passes `max` bytes before the next newline.
/// rmcp's TCP reader has no line cap of its own.
pub(super) struct CappedLineRead<R> {
    inner: R,
    max: usize,
    since_nl: usize,
    failed: bool,
    flag: Arc<AtomicBool>,
}

impl<R> CappedLineRead<R> {
    pub(super) fn new(inner: R, max: usize, flag: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            max,
            since_nl: 0,
            failed: false,
            flag,
        }
    }

    fn fail(&mut self) {
        self.failed = true;
        self.flag.store(true, Ordering::Release);
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for CappedLineRead<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(Err(line_too_long()));
        }
        let room = if this.since_nl >= this.max {
            1
        } else {
            this.max - this.since_nl
        };
        let want = room.min(buf.remaining());
        if want == 0 {
            return Poll::Ready(Ok(()));
        }
        let before = buf.filled().len();
        let mut tmp = vec![0u8; want];
        let n = {
            let mut limited = ReadBuf::new(&mut tmp);
            match Pin::new(&mut this.inner).poll_read(cx, &mut limited) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Ready(Ok(())) => limited.filled().len(),
            }
        };
        if n == 0 {
            return Poll::Ready(Ok(()));
        }
        buf.initialize_unfilled_to(n).copy_from_slice(&tmp[..n]);
        buf.advance(n);
        let new = &buf.filled()[before..];
        let mut count = this.since_nl;
        let mut overflow = None;
        for (index, byte) in new.iter().enumerate() {
            if *byte == b'\n' {
                count = 0;
            } else {
                count += 1;
                if count > this.max {
                    overflow = Some(index);
                    break;
                }
            }
        }
        if let Some(index) = overflow {
            buf.set_filled(before + index);
            this.fail();
            if index == 0 {
                return Poll::Ready(Err(line_too_long()));
            }
            this.since_nl = this.max;
            return Poll::Ready(Ok(()));
        }
        this.since_nl = count;
        Poll::Ready(Ok(()))
    }
}

fn line_too_long() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "line too long")
}

/// One token per accepted line. A refusal leaves the bucket unchanged.
pub(super) struct TokenBucket {
    rate_per_sec: u128,
    burst: u128,
    tokens: u128,
    updated: Instant,
}

impl TokenBucket {
    pub(super) fn new(rate_per_sec: u64, burst: u64) -> Self {
        let burst = u128::from(burst);
        Self {
            rate_per_sec: u128::from(rate_per_sec),
            burst,
            tokens: burst * SCALE,
            updated: Instant::now(),
        }
    }

    pub(super) fn allow(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.updated);
        self.updated = now;
        let refill = elapsed.as_nanos().saturating_mul(self.rate_per_sec * SCALE) / 1_000_000_000;
        self.tokens = self.tokens.saturating_add(refill).min(self.burst * SCALE);
        if self.tokens >= SCALE {
            self.tokens -= SCALE;
            true
        } else {
            false
        }
    }
}

pub(super) async fn write_frames<W>(
    mut write: W,
    mut frames: tokio::sync::mpsc::Receiver<Vec<u8>>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    flushed: Arc<AtomicUsize>,
) where
    W: AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    loop {
        let frame = tokio::select! {
            biased;
            frame = frames.recv() => frame,
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break;
                }
                continue;
            }
        };
        let Some(frame) = frame else {
            break;
        };
        let pending = write_all_flush(&mut write, &frame);
        tokio::pin!(pending);
        tokio::select! {
            result = &mut pending => {
                if result.is_err() {
                    break;
                }
                flushed.fetch_add(1, Ordering::Release);
            }
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break;
                }
            }
        }
    }
    let _ = write.shutdown().await;
}

async fn write_all_flush(write: &mut (impl AsyncWrite + Unpin), frame: &[u8]) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    write.write_all(frame).await?;
    write.flush().await?;
    Ok(())
}

/// Waits until `counter` moves, or one second passes.
pub(super) async fn wait_flushed(counter: &AtomicUsize, seen: usize) {
    let start = Instant::now();
    while counter.load(Ordering::Acquire) == seen {
        if start.elapsed() > Duration::from_secs(1) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use tokio::io::{AsyncReadExt, BufReader};

    use super::*;

    #[test]
    fn a_bucket_allows_the_burst_then_refuses() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(RATE_PER_SEC, BURST);
        for _ in 0..BURST {
            assert!(bucket.allow(start));
        }
        assert!(!bucket.allow(start));
        assert!(!bucket.allow(start));
    }

    #[test]
    fn a_bucket_refills_and_a_refusal_does_not_spend_a_token() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, 10);
        for _ in 0..10 {
            assert!(bucket.allow(start));
        }
        assert!(!bucket.allow(start));
        assert!(!bucket.allow(start));
        let later = start + Duration::from_millis(200);
        assert!(bucket.allow(later));
        assert!(!bucket.allow(later));
        let after = later + Duration::from_secs(1);
        for _ in 0..5 {
            assert!(bucket.allow(after));
        }
        assert!(!bucket.allow(after));
    }

    #[test]
    fn a_bucket_does_not_refill_from_an_earlier_instant() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, 1);
        assert!(bucket.allow(start + Duration::from_secs(1)));
        assert!(!bucket.allow(start));
    }

    #[tokio::test]
    async fn a_line_at_the_cap_is_kept_and_one_more_byte_is_not() {
        let mut exact = vec![b'a'; PLAYER_LINE_CAP];
        exact.push(b'\n');
        let mut reader = BufReader::new(Cursor::new(exact));
        let line = read_capped_line(&mut reader, PLAYER_LINE_CAP)
            .await
            .expect("read")
            .expect("line");
        assert_eq!(line.len(), PLAYER_LINE_CAP);

        let mut over = vec![b'a'; PLAYER_LINE_CAP + 1];
        over.push(b'\n');
        let mut reader = BufReader::new(Cursor::new(over));
        let err = read_capped_line(&mut reader, PLAYER_LINE_CAP)
            .await
            .expect_err("cap");
        assert!(matches!(err, LineFail::TooLong));
    }

    #[tokio::test]
    async fn crlf_eof_and_invalid_utf8() {
        let mut reader = BufReader::new(Cursor::new(&b"hi\r\n"[..]));
        let line = read_capped_line(&mut reader, 8).await.unwrap().unwrap();
        assert_eq!(line, "hi");

        let mut reader = BufReader::new(Cursor::new(&b"partial"[..]));
        let line = read_capped_line(&mut reader, 8).await.unwrap().unwrap();
        assert_eq!(line, "partial");

        let mut reader = BufReader::new(Cursor::new(&b""[..]));
        assert!(read_capped_line(&mut reader, 8).await.unwrap().is_none());

        let mut reader = BufReader::new(Cursor::new(&b"\xff\n"[..]));
        let err = read_capped_line(&mut reader, 8).await.unwrap_err();
        assert!(matches!(err, LineFail::BadUtf8));
    }

    #[tokio::test]
    async fn a_hello_read_stops_at_the_newline() {
        let data = b"gm\n{\"jsonrpc\":1}\n";
        let mut reader = Unbuffered::new(Cursor::new(&data[..]));
        let line = read_capped_line(&mut reader, PLAYER_LINE_CAP)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(line, "gm");
        assert_eq!(reader.inner.position(), 3);
    }

    #[tokio::test]
    async fn a_capped_reader_stops_near_the_max() {
        let flag = Arc::new(AtomicBool::new(false));
        let data = vec![b'x'; 4096];
        let mut reader = CappedLineRead::new(Cursor::new(data), 1024, Arc::clone(&flag));
        let mut sink = [0u8; 100];
        let mut total = 0usize;
        let err = loop {
            match reader.read(&mut sink).await {
                Ok(0) => panic!("eof before the cap"),
                Ok(n) => total += n,
                Err(err) => break err,
            }
        };
        assert!(total <= 1024, "{total}");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(flag.load(Ordering::Acquire));
        assert!(reader.inner.position() <= 1025);
    }
}
