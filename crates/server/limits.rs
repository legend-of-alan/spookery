//! Caps, the hello deadline, and the per-player token bucket.
//!
//! A hello is read one byte at a time so a following line stays on the socket
//! for the player reader. Player lines after the hello use a `BufReader`.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufRead, AsyncRead, AsyncWrite, ReadBuf};

/// Hello and player lines, not counting the newline.
pub(super) const PLAYER_LINE_CAP: usize = 8 * 1024;

/// HTTP POST body cap for the GM seat.
pub(super) const GM_BODY_CAP: usize = 1024 * 1024;

/// Player lines accepted per second after the burst is spent.
pub(super) const RATE_PER_SEC: u64 = 5;

/// Player lines accepted immediately on a fresh connection.
pub(super) const BURST: u64 = 10;

/// Frames queued toward one socket before that socket is dropped.
pub const OUTBOUND_FRAMES: usize = 256;

/// Bytes queued toward one socket before that socket is dropped.
/// A 256-frame queue of large says fills the tape before a large TCP
/// window fills, so the byte cap is what drops a stopped reader there.
pub const OUTBOUND_BYTES: usize = 64 * 1024;

/// Send buffer the player socket asks for. The OS may grant more.
pub const REQUESTED_SEND_BUFFER: usize = 8 * 1024;

pub const HELLO_LIMIT: Duration = Duration::from_secs(10);

/// Millitokens per line. Refill uses this scale so the bucket stays integer.
const SCALE: u128 = 1000;

pub(super) enum Outbound {
    Frame(Vec<u8>),
    /// Flushed, then acknowledged, then the writer shuts the socket down.
    Last(Vec<u8>, tokio::sync::oneshot::Sender<()>),
}

pub(super) type CancelTx = tokio::sync::watch::Sender<bool>;

/// Sender half of one socket's outbound queue.
#[derive(Clone)]
pub(super) struct OutTx {
    tx: tokio::sync::mpsc::Sender<Outbound>,
    queued: Arc<AtomicUsize>,
}

/// Receiver half. A frame's bytes stay counted until it is written.
pub(super) struct OutRx {
    rx: tokio::sync::mpsc::Receiver<Outbound>,
    queued: Arc<AtomicUsize>,
}

impl OutTx {
    fn enqueue(&self, item: Outbound) -> bool {
        let len = outbound_len(&item);
        if !self.reserve(len) {
            return false;
        }
        if self.tx.try_send(item).is_err() {
            release(&self.queued, len);
            return false;
        }
        true
    }

    /// An empty queue takes one frame of any size, so a welcome still fits.
    fn reserve(&self, len: usize) -> bool {
        self.queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
                if cur == 0 {
                    return Some(len);
                }
                let next = cur.saturating_add(len);
                if next > OUTBOUND_BYTES {
                    None
                } else {
                    Some(next)
                }
            })
            .is_ok()
    }
}

impl OutRx {
    pub(super) async fn recv(&mut self) -> Option<Outbound> {
        self.rx.recv().await
    }

    /// Releases a frame's bytes once the writer has sent it.
    pub(super) fn release_written(&self, frame: &[u8]) {
        release(&self.queued, frame.len());
    }
}

fn outbound_len(item: &Outbound) -> usize {
    match item {
        Outbound::Frame(bytes) | Outbound::Last(bytes, _) => bytes.len(),
    }
}

fn release(queued: &AtomicUsize, len: usize) {
    let _ = queued.fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
        Some(cur.saturating_sub(len))
    });
}

pub(super) fn outbound() -> (OutTx, OutRx, CancelTx, tokio::sync::watch::Receiver<bool>) {
    let (tx, rx) = tokio::sync::mpsc::channel(OUTBOUND_FRAMES);
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let queued = Arc::new(AtomicUsize::new(0));
    (
        OutTx {
            tx,
            queued: Arc::clone(&queued),
        },
        OutRx { rx, queued },
        cancel_tx,
        cancel_rx,
    )
}

pub(super) fn try_frame(tx: &OutTx, bytes: Vec<u8>) -> bool {
    tx.enqueue(Outbound::Frame(bytes))
}

/// Queues the closing frame and waits until the writer has flushed it.
pub(super) async fn send_last(tx: &OutTx, frame: Vec<u8>) {
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    if tx.enqueue(Outbound::Last(frame, ack_tx)) {
        let _ = ack_rx.await;
    }
}

#[derive(Debug)]
pub(super) enum LineReadError {
    Io,
    TooLong,
    BadUtf8,
}

/// One LF line. `cap` counts every byte before the LF, including a CR.
/// The buffer never holds more than `cap` bytes.
pub(super) async fn read_capped_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    cap: usize,
) -> Result<Option<String>, LineReadError> {
    use tokio::io::AsyncBufReadExt;
    let mut buf = Vec::new();
    loop {
        if buf.len() > cap {
            return Err(LineReadError::TooLong);
        }
        let room = cap - buf.len();
        let (take, finished, too_long, eof) = {
            let filled = reader.fill_buf().await.map_err(|_| LineReadError::Io)?;
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
            let filled = reader.fill_buf().await.map_err(|_| LineReadError::Io)?;
            buf.extend_from_slice(&filled[..take]);
            reader.consume(take);
        }
        if too_long {
            return Err(LineReadError::TooLong);
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

fn finish(buf: Vec<u8>) -> Result<Option<String>, LineReadError> {
    match String::from_utf8(buf) {
        Ok(text) => Ok(Some(text)),
        Err(_) => Err(LineReadError::BadUtf8),
    }
}

/// Reads one byte at a time and only the bytes the line reader consumes.
pub(super) struct ByteReader<R> {
    inner: R,
    buf: [u8; 1],
    len: usize,
}

impl<R> ByteReader<R> {
    pub(super) fn new(inner: R) -> Self {
        Self {
            inner,
            buf: [0; 1],
            len: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for ByteReader<R> {
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

impl<R: AsyncRead + Unpin> AsyncBufRead for ByteReader<R> {
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

/// One token per line that is read. An empty bucket spends nothing.
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

    /// Spends one token when one is available.
    pub(super) fn allow(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.tokens >= SCALE {
            self.tokens -= SCALE;
            true
        } else {
            false
        }
    }

    /// How long until one line token is available. Zero means one is ready.
    pub(super) fn until_token(&mut self, now: Instant) -> Duration {
        self.refill(now);
        if self.tokens >= SCALE {
            return Duration::ZERO;
        }
        if self.rate_per_sec == 0 {
            return Duration::from_secs(60);
        }
        let need = SCALE - self.tokens;
        let per_ns = self.rate_per_sec.saturating_mul(SCALE);
        let need_ns = need.saturating_mul(1_000_000_000).div_ceil(per_ns);
        let residual = now.saturating_duration_since(self.updated).as_nanos();
        let ns = need_ns.saturating_sub(residual);
        Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
    }

    fn refill(&mut self, now: Instant) {
        let cap = self.burst.saturating_mul(SCALE);
        if self.rate_per_sec == 0 || self.tokens >= cap {
            self.updated = now;
            return;
        }
        let elapsed_ns = now.saturating_duration_since(self.updated).as_nanos();
        let per_ns = self.rate_per_sec.saturating_mul(SCALE);
        let refill = elapsed_ns.saturating_mul(per_ns) / 1_000_000_000;
        if refill == 0 {
            return;
        }
        let added = refill.min(cap - self.tokens);
        let consumed_ns = added.saturating_mul(1_000_000_000) / per_ns;
        self.tokens = self.tokens.saturating_add(added);
        self.updated += Duration::from_nanos(u64::try_from(consumed_ns).unwrap_or(u64::MAX));
    }
}

/// Waits until a line token is ready. False means the seat was dropped.
pub(super) async fn wait_turn(
    bucket: &mut TokenBucket,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    loop {
        if *cancel.borrow() {
            return false;
        }
        let delay = bucket.until_token(Instant::now());
        if delay.is_zero() {
            return true;
        }
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    return false;
                }
            }
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

pub(super) async fn write_frames<W>(
    mut write: W,
    mut frames: OutRx,
    mut cancel: tokio::sync::watch::Receiver<bool>,
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
        match frame {
            Outbound::Frame(bytes) => {
                let pending = write_all_flush(&mut write, &bytes);
                tokio::pin!(pending);
                tokio::select! {
                    biased;
                    result = &mut pending => {
                        if result.is_err() {
                            break;
                        }
                        frames.release_written(&bytes);
                    }
                    changed = cancel.changed() => {
                        if changed.is_err() || *cancel.borrow() {
                            break;
                        }
                    }
                }
            }
            Outbound::Last(bytes, ack) => {
                let _ = write_all_flush(&mut write, &bytes).await;
                let _ = ack.send(());
                break;
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use tokio::io::BufReader;

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

    #[test]
    fn a_bucket_keeps_time_that_did_not_fill_a_millitoken() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, 1);
        assert!(bucket.allow(start));
        assert!(!bucket.allow(start));
        let mut now = start;
        for _ in 0..1999 {
            now += Duration::from_micros(100);
            assert!(
                !bucket.allow(now),
                "{:?}",
                now.saturating_duration_since(start)
            );
        }
        now += Duration::from_micros(100);
        assert!(bucket.allow(now));
    }

    #[test]
    fn a_bucket_waits_only_for_the_missing_token() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, 1);
        assert!(bucket.allow(start));
        let full = bucket.until_token(start);
        assert_eq!(full, Duration::from_millis(200));
        let later = start + Duration::from_micros(100);
        assert!(!bucket.allow(later));
        let rest = bucket.until_token(later);
        assert!(rest < full, "{rest:?}");
        assert!(rest > Duration::from_millis(199), "{rest:?}");
    }

    #[tokio::test]
    async fn the_last_frame_is_flushed_before_its_ack() {
        use tokio::io::AsyncReadExt;
        let (mut client, server) = tokio::io::duplex(256);
        let (tx, rx, _cancel_tx, cancel_rx) = outbound();
        let writer = tokio::spawn(write_frames(server, rx, cancel_rx));
        assert!(try_frame(&tx, b"one\n".to_vec()));
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        assert!(tx.enqueue(Outbound::Last(b"bye\n".to_vec(), ack_tx)));
        ack_rx.await.expect("ack");
        drop(tx);
        writer.await.expect("writer");
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.expect("read");
        assert_eq!(buf, b"one\nbye\n");
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
        assert!(matches!(err, LineReadError::TooLong));
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
        assert!(matches!(err, LineReadError::BadUtf8));
    }

    #[tokio::test]
    async fn a_hello_read_stops_at_the_newline() {
        let data = b"player:ada\n{\"jsonrpc\":1}\n";
        let mut reader = ByteReader::new(Cursor::new(&data[..]));
        let line = read_capped_line(&mut reader, PLAYER_LINE_CAP)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(line, "player:ada");
        assert_eq!(reader.inner.position(), 11);
    }

    #[test]
    fn the_frame_cap_rejects_before_the_byte_cap() {
        let (tx, _rx, _cancel_tx, _cancel_rx) = outbound();
        for _ in 0..OUTBOUND_FRAMES {
            assert!(try_frame(&tx, b"n".to_vec()));
        }
        assert!(!try_frame(&tx, b"n".to_vec()));
    }

    #[tokio::test]
    async fn the_byte_cap_rejects_until_a_frame_is_written() {
        let (tx, mut rx, _cancel_tx, _cancel_rx) = outbound();
        assert!(try_frame(&tx, vec![b'a'; OUTBOUND_BYTES + 8]));
        assert!(
            !try_frame(&tx, b"n".to_vec()),
            "a second frame was queued past the byte cap"
        );
        let taken = rx.recv().await.expect("frame");
        let Outbound::Frame(bytes) = taken else {
            panic!("expected a frame");
        };
        assert_eq!(bytes.len(), OUTBOUND_BYTES + 8);
        assert!(
            !try_frame(&tx, b"n".to_vec()),
            "a taken frame was released before it was written"
        );
        rx.release_written(&bytes);
        assert!(try_frame(&tx, b"n".to_vec()));
    }
}
