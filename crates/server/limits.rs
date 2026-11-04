//! Caps, socket options and the per-connection token bucket.

use std::time::Duration;

use tokio::time::Instant;

/// The longest client message the table reads. A longer one is `LINE_TOO_LONG`.
pub(super) const CLIENT_MESSAGE_CAP: usize = 8 * 1024;

/// The longest WebSocket message the game port takes in at all. A longer one drops the
/// connection.
pub(super) const MESSAGE_CAP: usize = 64 * 1024;

/// HTTP POST body cap for the GM seat.
pub(super) const GM_BODY_CAP: usize = 1024 * 1024;

/// Client messages accepted per second after the burst is spent.
pub(super) const RATE_PER_SEC: u64 = 5;

/// Client messages accepted immediately on a fresh connection.
pub(super) const BURST: u64 = 10;

/// Frames queued toward one socket before that socket is dropped.
pub(super) const OUTBOUND_FRAME_CAP: usize = 256;

/// How many messages a claim keeps for a reconnect. Older ones are dropped, and the
/// `claimed` reply says `gap`.
pub(super) const MISSED_MESSAGE_CAP: usize = 256;

/// Bytes queued toward one socket before that socket is dropped.
/// A 256-frame queue of large says fills the tape before a large TCP
/// window fills, so the byte cap is what drops a stopped reader there.
pub(super) const OUTBOUND_BYTE_CAP: usize = 64 * 1024;

/// Send buffer a game port socket asks for. The OS may grant more.
pub(super) const REQUESTED_SEND_BUFFER: usize = 8 * 1024;

/// Keepalive probes on a game port socket: the first after 30 idle seconds, then
/// three more one second apart. A silent socket holds its seat for about 33 seconds.
pub(super) const KEEPALIVE: socket2::TcpKeepalive = socket2::TcpKeepalive::new()
    .with_time(Duration::from_secs(30))
    .with_interval(Duration::from_secs(1))
    .with_retries(3);

/// How long a new connection may wait before its first message.
pub(super) const FIRST_MESSAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// Millitokens per message. Refill uses this scale so the bucket stays integer.
const MILLITOKENS_PER_TOKEN: u128 = 1000;

/// One token per message that is read. An empty bucket spends nothing.
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
    cancel: &mut tokio::sync::watch::Receiver<()>,
) -> bool {
    loop {
        if cancel.has_changed().is_err() {
            return false;
        }
        let delay = bucket.until_token(Instant::now());
        if delay.is_zero() {
            return true;
        }
        tokio::select! {
            biased;
            _ = cancel.changed() => return false,
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

#[cfg(test)]
mod tests {
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

    #[tokio::test(start_paused = true)]
    async fn a_dropped_seat_ends_the_wait_for_a_token_at_once() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, 1);
        assert!(bucket.try_take(start));
        let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(());
        let (waited, ()) = tokio::join!(wait_for_token(&mut bucket, &mut cancel_rx), async {
            drop(cancel_tx);
        });
        assert!(!waited);
        let mut ready = TokenBucket::new(5, 1);
        assert!(!wait_for_token(&mut ready, &mut cancel_rx).await);
        assert_eq!(Instant::now(), start);
    }
}
