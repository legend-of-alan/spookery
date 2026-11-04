//! Who is connected and seated, each claim and the messages it keeps for a reconnect, and
//! each connection's capped outbound queue and its writer.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use spookery_engine::Presence;
use tokio::sync::{mpsc, watch};

use super::limits::{MISSED_MESSAGE_CAP, OUTBOUND_BYTE_CAP, OUTBOUND_FRAME_CAP};
use crate::protocol::{Seat, ServerMessage};

/// One connection's id, on the game port or the MCP endpoint.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Conn(u64);

impl Conn {
    /// The next id from the one counter both ports share.
    pub(crate) fn new(next: &AtomicU64) -> Self {
        Self(next.fetch_add(1, Ordering::Relaxed))
    }
}

/// The one owner of who is connected and who is seated. Rounds and readers take
/// `presence()`.
#[derive(Default)]
pub(super) struct Seats {
    pub(super) connections: Vec<Connection>,
    /// The connection that holds the GM seat.
    pub(super) gm: Option<Conn>,
    /// Each held claim, oldest first. At most three, so a scan finds one.
    pub(super) claims: Vec<Claim>,
}

impl Seats {
    /// The claimed PCs, sorted, so a round records the same `actors_present` bytes. A claim
    /// whose connection dropped is still present.
    pub(super) fn presence(&self) -> Presence {
        let mut actors_present: Vec<String> = self
            .claims
            .iter()
            .map(|claim| claim.actor.clone())
            .collect();
        actors_present.sort();
        Presence {
            players: u32::try_from(actors_present.len()).unwrap_or(u32::MAX),
            actors_present,
        }
    }

    /// The claim `conn` drives.
    pub(super) fn claim_of(&self, conn: Conn) -> Option<&Claim> {
        self.claims.iter().find(|claim| claim.conn == Some(conn))
    }

    pub(super) fn claim_of_mut(&mut self, conn: Conn) -> Option<&mut Claim> {
        self.claims
            .iter_mut()
            .find(|claim| claim.conn == Some(conn))
    }

    /// The claim on the PC `actor`.
    pub(super) fn claim_of_actor_mut(&mut self, actor: &str) -> Option<&mut Claim> {
        self.claims.iter_mut().find(|claim| claim.actor == actor)
    }

    pub(super) fn connection(&self, conn: Conn) -> Option<&Connection> {
        self.connections
            .iter()
            .find(|connection| connection.conn == conn)
    }

    /// Drops the connection. Dropping it cancels its writer.
    pub(super) fn take(&mut self, conn: Conn) -> Option<Connection> {
        let index = self
            .connections
            .iter()
            .position(|connection| connection.conn == conn)?;
        Some(self.connections.remove(index))
    }
}

/// The table's entry for one open `Conn`: which seat it opened and its outbound queue.
pub(super) struct Connection {
    pub(super) conn: Conn,
    pub(super) seat: Seat,
    pub(super) out: FrameSender,
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.out.cancel.send(true);
    }
}

/// One player's hold on a PC. It outlives its connection and ends only on `release`. A
/// claim by its `token` takes it back, with the messages it kept.
pub(super) struct Claim {
    pub(super) token: String,
    pub(super) actor: String,
    /// The connection that drives it, while one does.
    pub(super) conn: Option<Conn>,
    /// The newest `MISSED_MESSAGE_CAP` messages sent to this claim, oldest first.
    kept: VecDeque<ServerMessage>,
    /// The `seq` of the newest message dropped past `MISSED_MESSAGE_CAP`, or 0.
    last_lost_seq: u64,
}

impl Claim {
    pub(super) fn new(token: String, actor: String) -> Self {
        Self {
            token,
            actor,
            conn: None,
            kept: VecDeque::new(),
            last_lost_seq: 0,
        }
    }

    /// Keeps `message` for a reconnect. The oldest is dropped past `MISSED_MESSAGE_CAP`.
    pub(super) fn keep(&mut self, message: &ServerMessage) {
        if self.kept.len() == MISSED_MESSAGE_CAP {
            if let Some(lost) = self.kept.pop_front() {
                self.last_lost_seq = lost.seq.unwrap_or(self.last_lost_seq);
            }
        }
        self.kept.push_back(message.clone());
    }

    /// The kept messages after `last_seq`, and whether any were dropped past `MISSED_MESSAGE_CAP`.
    pub(super) fn messages_after(&self, last_seq: u64) -> (Vec<ServerMessage>, bool) {
        let missed = self
            .kept
            .iter()
            .filter(|message| message.seq.is_some_and(|seq| seq > last_seq))
            .cloned()
            .collect();
        (missed, last_seq < self.last_lost_seq)
    }
}

/// The table's end of one seat's frame queue. The table sends that seat frames with it.
/// Also holds the seat's cancel.
#[derive(Clone)]
pub(super) struct FrameSender {
    tx: mpsc::Sender<Vec<u8>>,
    queued_bytes: Arc<AtomicUsize>,
    cancel: watch::Sender<bool>,
}

/// The writer's end of one seat's frame queue. That seat's socket or event writer reads from it.
/// A frame's bytes stay counted until it is written.
pub(super) struct FrameReceiver {
    rx: mpsc::Receiver<Vec<u8>>,
    queued_bytes: Arc<AtomicUsize>,
    pub(super) cancel: watch::Receiver<bool>,
}

impl FrameSender {
    /// Queues one frame. False when the queue is full or closed.
    pub(super) fn send(&self, frame: Vec<u8>) -> bool {
        let len = frame.len();
        if !self.reserve(len) {
            return false;
        }
        if self.tx.try_send(frame).is_err() {
            release(&self.queued_bytes, len);
            return false;
        }
        true
    }

    /// An empty queue takes one frame of any size, so a welcome still fits.
    fn reserve(&self, len: usize) -> bool {
        self.queued_bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
                if cur == 0 {
                    return Some(len);
                }
                let next = cur.saturating_add(len);
                if next > OUTBOUND_BYTE_CAP {
                    None
                } else {
                    Some(next)
                }
            })
            .is_ok()
    }
}

impl FrameReceiver {
    /// The next frame, or `None` once the queue is closed and empty. Its bytes are released
    /// as it is handed on, for a writer that cannot see the send.
    pub(super) fn poll_frame(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<u8>>> {
        let frame = ready!(self.rx.poll_recv(cx));
        if let Some(bytes) = &frame {
            release(&self.queued_bytes, bytes.len());
        }
        Poll::Ready(frame)
    }
}

fn release(queued: &AtomicUsize, len: usize) {
    queued.update(Ordering::AcqRel, Ordering::Acquire, |cur| {
        cur.saturating_sub(len)
    });
}

/// One seat's frame queue. Like `mpsc::channel()`, it returns the sender and
/// receiver halves. A frame is counted against `OUTBOUND_FRAME_CAP` and
/// `OUTBOUND_BYTE_CAP` from the moment it is queued until the writer has sent
/// it.
pub(super) fn frame_queue() -> (FrameSender, FrameReceiver) {
    let (tx, rx) = mpsc::channel(OUTBOUND_FRAME_CAP);
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let queued_bytes = Arc::new(AtomicUsize::new(0));
    (
        FrameSender {
            tx,
            queued_bytes: Arc::clone(&queued_bytes),
            cancel: cancel_tx,
        },
        FrameReceiver {
            rx,
            queued_bytes,
            cancel: cancel_rx,
        },
    )
}

/// Sends each queued frame to `sink` until the queue closes, the seat is
/// cancelled, or `sink` fails. A frame's bytes are released once it is sent.
pub(super) async fn write_frames(
    mut sink: impl AsyncFnMut(&[u8]) -> bool,
    mut frames_rx: FrameReceiver,
) {
    loop {
        let frame = tokio::select! {
            biased;
            frame = frames_rx.rx.recv() => frame,
            _ = frames_rx.cancel.changed() => break,
        };
        let Some(bytes) = frame else {
            break;
        };
        tokio::select! {
            biased;
            sent = sink(&bytes) => {
                if !sent {
                    break;
                }
                release(&frames_rx.queued_bytes, bytes.len());
            }
            _ = frames_rx.cancel.changed() => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_cap_rejects_before_the_byte_cap() {
        let (frames_tx, _frames_rx) = frame_queue();
        for _ in 0..OUTBOUND_FRAME_CAP {
            assert!(frames_tx.send(b"n".to_vec()));
        }
        assert!(!frames_tx.send(b"n".to_vec()));
    }

    #[tokio::test]
    async fn the_byte_cap_rejects_until_a_frame_is_written() {
        let (frames_tx, mut frames_rx) = frame_queue();
        assert!(frames_tx.send(vec![b'a'; OUTBOUND_BYTE_CAP + 8]));
        assert!(
            !frames_tx.send(b"n".to_vec()),
            "a second frame was queued past the byte cap"
        );
        let bytes = frames_rx.rx.recv().await.expect("frame");
        assert_eq!(bytes.len(), OUTBOUND_BYTE_CAP + 8);
        assert!(
            !frames_tx.send(b"n".to_vec()),
            "a taken frame was released before it was written"
        );
        release(&frames_rx.queued_bytes, bytes.len());
        assert!(frames_tx.send(b"n".to_vec()));
    }
}
