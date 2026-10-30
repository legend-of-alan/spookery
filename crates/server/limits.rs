//! Caps, the hello deadline, and the per-player token bucket.
//!
//! A hello is read one byte at a time so a following line stays on the socket
//! for the player reader. Player lines after the hello use a `BufReader`.

use std::io;
use std::pin::Pin;
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
pub const OUTBOUND_FRAME_CAP: usize = 256;

/// Bytes queued toward one socket before that socket is dropped.
/// A 256-frame queue of large says fills the tape before a large TCP
/// window fills, so the byte cap is what drops a stopped reader there.
pub const OUTBOUND_BYTE_CAP: usize = 64 * 1024;

/// Send buffer the player socket asks for. The OS may grant more.
pub const REQUESTED_SEND_BUFFER: usize = 8 * 1024;

pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// Millitokens per line. Refill uses this scale so the bucket stays integer.
const MILLITOKENS_PER_TOKEN: u128 = 1000;

#[derive(Debug)]
pub(super) enum ReadLineError {
    Io,
    TooLong,
    BadUtf8,
}

/// One LF line. `cap` counts every byte before the LF, including a CR.
/// The buffer never holds more than `cap` bytes.
pub(super) async fn read_capped_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    cap: usize,
) -> Result<Option<String>, ReadLineError> {
    use tokio::io::AsyncBufReadExt;
    let mut buf = Vec::new();
    loop {
        if buf.len() > cap {
            return Err(ReadLineError::TooLong);
        }
        let room = cap - buf.len();
        let (take, finished, too_long, eof) = {
            let filled = reader.fill_buf().await.map_err(|_| ReadLineError::Io)?;
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
            let filled = reader.fill_buf().await.map_err(|_| ReadLineError::Io)?;
            buf.extend_from_slice(&filled[..take]);
            reader.consume(take);
        }
        if too_long {
            return Err(ReadLineError::TooLong);
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

fn finish(buf: Vec<u8>) -> Result<Option<String>, ReadLineError> {
    match String::from_utf8(buf) {
        Ok(text) => Ok(Some(text)),
        Err(_) => Err(ReadLineError::BadUtf8),
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
            tokens: burst * MILLITOKENS_PER_TOKEN,
            updated: Instant::now(),
        }
    }

    /// Spends one token when one is available.
    pub(super) fn try_take(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.tokens >= MILLITOKENS_PER_TOKEN {
            self.tokens -= MILLITOKENS_PER_TOKEN;
            true
        } else {
            false
        }
    }

    /// How long until one line token is available. Zero means one is ready.
    pub(super) fn until_token(&mut self, now: Instant) -> Duration {
        self.refill(now);
        if self.tokens >= MILLITOKENS_PER_TOKEN {
            return Duration::ZERO;
        }
        if self.rate_per_sec == 0 {
            return Duration::from_secs(60);
        }
        let need = MILLITOKENS_PER_TOKEN - self.tokens;
        let per_ns = self.rate_per_sec.saturating_mul(MILLITOKENS_PER_TOKEN);
        let need_ns = need.saturating_mul(1_000_000_000).div_ceil(per_ns);
        let residual = now.saturating_duration_since(self.updated).as_nanos();
        let ns = need_ns.saturating_sub(residual);
        Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
    }

    fn refill(&mut self, now: Instant) {
        let cap = self.burst.saturating_mul(MILLITOKENS_PER_TOKEN);
        if self.rate_per_sec == 0 || self.tokens >= cap {
            self.updated = now;
            return;
        }
        let elapsed_ns = now.saturating_duration_since(self.updated).as_nanos();
        let per_ns = self.rate_per_sec.saturating_mul(MILLITOKENS_PER_TOKEN);
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
pub(super) async fn wait_for_token(
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

pub(super) async fn write_all_flush(write: &mut (impl AsyncWrite + Unpin), frame: &[u8]) -> bool {
    use tokio::io::AsyncWriteExt;
    write.write_all(frame).await.is_ok() && write.flush().await.is_ok()
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
            assert!(bucket.try_take(start));
        }
        assert!(!bucket.try_take(start));
        assert!(!bucket.try_take(start));
    }

    #[test]
    fn a_bucket_refills_and_a_refusal_does_not_spend_a_token() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, 10);
        for _ in 0..10 {
            assert!(bucket.try_take(start));
        }
        assert!(!bucket.try_take(start));
        assert!(!bucket.try_take(start));
        let later = start + Duration::from_millis(200);
        assert!(bucket.try_take(later));
        assert!(!bucket.try_take(later));
        let after = later + Duration::from_secs(1);
        for _ in 0..5 {
            assert!(bucket.try_take(after));
        }
        assert!(!bucket.try_take(after));
    }

    #[test]
    fn a_bucket_does_not_refill_from_an_earlier_instant() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, 1);
        assert!(bucket.try_take(start + Duration::from_secs(1)));
        assert!(!bucket.try_take(start));
    }

    #[test]
    fn a_bucket_keeps_time_that_did_not_fill_a_millitoken() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, 1);
        assert!(bucket.try_take(start));
        assert!(!bucket.try_take(start));
        let mut now = start;
        for _ in 0..1999 {
            now += Duration::from_micros(100);
            assert!(
                !bucket.try_take(now),
                "{:?}",
                now.saturating_duration_since(start)
            );
        }
        now += Duration::from_micros(100);
        assert!(bucket.try_take(now));
    }

    #[test]
    fn a_bucket_waits_only_for_the_missing_token() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, 1);
        assert!(bucket.try_take(start));
        let full = bucket.until_token(start);
        assert_eq!(full, Duration::from_millis(200));
        let later = start + Duration::from_micros(100);
        assert!(!bucket.try_take(later));
        let rest = bucket.until_token(later);
        assert!(rest < full, "{rest:?}");
        assert!(rest > Duration::from_millis(199), "{rest:?}");
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
        assert!(matches!(err, ReadLineError::TooLong));
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
        assert!(matches!(err, ReadLineError::BadUtf8));
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
}
