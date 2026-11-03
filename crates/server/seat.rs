//! Who is connected and seated, and each connection's capped outbound queue and its writer.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use spookery_engine::Presence;
use tokio::sync::{mpsc, watch};

use super::limits::{OUTBOUND_BYTE_CAP, OUTBOUND_FRAME_CAP};
use crate::protocol::Seat;

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
    pub(super) links: Vec<Link>,
    /// The connection that holds the GM seat.
    pub(super) gm: Option<Conn>,
}

impl Seats {
    /// The claimed PCs, sorted, so a round records the same `actors_present` bytes.
    pub(super) fn presence(&self) -> Presence {
        let mut actors_present: Vec<String> = self
            .players()
            .map(|(_, actor_id)| actor_id.to_string())
            .collect();
        actors_present.sort();
        Presence {
            players: u32::try_from(actors_present.len()).unwrap_or(u32::MAX),
            actors_present,
        }
    }

    /// Each player connection that holds a claim, with its PC.
    pub(super) fn players(&self) -> impl Iterator<Item = (&Link, &str)> {
        self.links
            .iter()
            .filter_map(|link| Some((link, link.actor.as_deref()?)))
    }

    pub(super) fn link(&self, conn: Conn) -> Option<&Link> {
        self.links.iter().find(|link| link.conn == conn)
    }

    pub(super) fn link_mut(&mut self, conn: Conn) -> Option<&mut Link> {
        self.links.iter_mut().find(|link| link.conn == conn)
    }

    /// Drops the connection. Dropping its link cancels its writer.
    pub(super) fn take(&mut self, conn: Conn) -> Option<Link> {
        let index = self.links.iter().position(|link| link.conn == conn)?;
        Some(self.links.remove(index))
    }
}

/// One open connection: which seat it opened, the PC it claims, and its outbound queue.
pub(super) struct Link {
    pub(super) conn: Conn,
    pub(super) seat: Seat,
    pub(super) actor: Option<String>,
    pub(super) out: FrameSender,
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.out.cancel.send(true);
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
