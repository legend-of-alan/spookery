//! Who is seated, and each seat's capped outbound queue and its writer.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use spookery_engine::Presence;
use tokio::io::AsyncWrite;
use tokio::sync::{mpsc, oneshot, watch};

use super::limits::{OUTBOUND_BYTE_CAP, OUTBOUND_FRAME_CAP};

/// One connection's id, on the game port or the agent door.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Conn(u64);

impl Conn {
    /// The next id from the one counter both ports share.
    pub(crate) fn mint(next: &AtomicU64) -> Self {
        Self(next.fetch_add(1, Ordering::Relaxed))
    }
}

/// The one owner of who is seated. Rounds and readers take `presence()`.
#[derive(Default)]
pub(super) struct Seats {
    pub(super) players: Vec<PlayerSeat>,
    pub(super) gm: Option<GmSeat>,
}

impl Seats {
    /// The seated PCs, sorted, so a round records the same `actors_present` bytes.
    pub(super) fn presence(&self) -> Presence {
        let mut actors_present: Vec<String> = self
            .players
            .iter()
            .map(|player| player.actor_id.clone())
            .collect();
        actors_present.sort();
        Presence {
            players: u32::try_from(actors_present.len()).unwrap_or(u32::MAX),
            actors_present,
        }
    }

    pub(super) fn player(&self, conn: Conn) -> Option<&PlayerSeat> {
        self.players.iter().find(|player| player.conn == conn)
    }

    pub(super) fn take_player(&mut self, conn: Conn) -> Option<PlayerSeat> {
        let index = self.players.iter().position(|player| player.conn == conn)?;
        Some(self.players.remove(index))
    }

    /// Dropping the seat cancels its event writer.
    pub(super) fn release_gm(&mut self) {
        self.gm = None;
    }
}

pub(super) struct PlayerSeat {
    pub(super) conn: Conn,
    pub(super) actor_id: String,
    pub(super) out: Outbound,
}

impl Drop for PlayerSeat {
    fn drop(&mut self) {
        let _ = self.out.cancel.send(true);
    }
}

pub(super) struct GmSeat {
    pub(super) conn: Conn,
    pub(super) out: Outbound,
}

impl Drop for GmSeat {
    fn drop(&mut self) {
        let _ = self.out.cancel.send(true);
    }
}

enum Message {
    Frame(Vec<u8>),
    /// Flushed, then acknowledged, then the writer shuts the socket down.
    Last(Vec<u8>, oneshot::Sender<()>),
}

/// Sender half of one seat's queue, and the seat's cancel.
#[derive(Clone)]
pub(super) struct Outbound {
    tx: mpsc::Sender<Message>,
    queued_bytes: Arc<AtomicUsize>,
    cancel: watch::Sender<bool>,
}

/// Receiver half. A frame's bytes stay counted until it is written.
pub(super) struct Inbound {
    rx: mpsc::Receiver<Message>,
    queued_bytes: Arc<AtomicUsize>,
    pub(super) cancel: watch::Receiver<bool>,
}

impl Message {
    fn len(&self) -> usize {
        match self {
            Message::Frame(bytes) | Message::Last(bytes, _) => bytes.len(),
        }
    }
}

impl Outbound {
    /// Queues one frame. False when the queue is full or closed.
    pub(super) fn send(&self, frame: Vec<u8>) -> bool {
        self.try_send(Message::Frame(frame))
    }

    /// Queues the closing frame and waits until the writer has flushed it.
    pub(super) async fn send_last(&self, frame: Vec<u8>) {
        let (ack_tx, ack_rx) = oneshot::channel();
        if self.try_send(Message::Last(frame, ack_tx)) {
            let _ = ack_rx.await;
        }
    }

    fn try_send(&self, item: Message) -> bool {
        let len = item.len();
        if !self.reserve(len) {
            return false;
        }
        if self.tx.try_send(item).is_err() {
            release(&self.queued_bytes, len);
            return false;
        }
        true
    }

    /// An empty queue takes one frame of any size, so a welcome still fits.
    fn reserve(&self, len: usize) -> bool {
        self.queued_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
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
    let _ = queued.fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
        Some(cur.saturating_sub(len))
    });
}

/// One seat's queue. A frame is counted against `OUTBOUND_FRAME_CAP` and
/// `OUTBOUND_BYTE_CAP` from the moment it is queued until the writer has sent
/// it. The hello keeps a clone of `Outbound` for `send_last`.
pub(super) fn seat() -> (Outbound, Inbound) {
    let (tx, rx) = mpsc::channel(OUTBOUND_FRAME_CAP);
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let queued_bytes = Arc::new(AtomicUsize::new(0));
    (
        Outbound {
            tx,
            queued_bytes: Arc::clone(&queued_bytes),
            cancel: cancel_tx,
        },
        Inbound {
            rx,
            queued_bytes,
            cancel: cancel_rx,
        },
    )
}

/// Sends each queued frame to `sink` until the queue closes, the seat is
/// cancelled, or `sink` fails. A frame's bytes are released once it is sent.
pub(super) async fn write_frames(mut sink: impl AsyncFnMut(&[u8]) -> bool, mut inbound: Inbound) {
    loop {
        let frame = tokio::select! {
            biased;
            frame = inbound.rx.recv() => frame,
            _ = inbound.cancel.changed() => break,
        };
        let Some(frame) = frame else {
            break;
        };
        match frame {
            Message::Frame(bytes) => {
                tokio::select! {
                    biased;
                    sent = sink(&bytes) => {
                        if !sent {
                            break;
                        }
                        release(&inbound.queued_bytes, bytes.len());
                    }
                    _ = inbound.cancel.changed() => break,
                }
            }
            Message::Last(bytes, ack) => {
                let _ = sink(&bytes).await;
                let _ = ack.send(());
                break;
            }
        }
    }
}

pub(super) async fn write_all_flush(write: &mut (impl AsyncWrite + Unpin), frame: &[u8]) -> bool {
    use tokio::io::AsyncWriteExt;
    write.write_all(frame).await.is_ok() && write.flush().await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_last_frame_is_flushed_before_its_ack() {
        use tokio::io::AsyncReadExt;
        let (mut client, mut server) = tokio::io::duplex(256);
        let (out, inbound) = seat();
        let writer = tokio::spawn(write_frames(
            async move |frame: &[u8]| write_all_flush(&mut server, frame).await,
            inbound,
        ));
        assert!(out.send(b"one\n".to_vec()));
        let (ack_tx, ack_rx) = oneshot::channel();
        assert!(out.try_send(Message::Last(b"bye\n".to_vec(), ack_tx)));
        ack_rx.await.expect("ack");
        drop(out);
        writer.await.expect("writer");
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.expect("read");
        assert_eq!(buf, b"one\nbye\n");
    }

    #[test]
    fn the_frame_cap_rejects_before_the_byte_cap() {
        let (out, _inbound) = seat();
        for _ in 0..OUTBOUND_FRAME_CAP {
            assert!(out.send(b"n".to_vec()));
        }
        assert!(!out.send(b"n".to_vec()));
    }

    #[tokio::test]
    async fn the_byte_cap_rejects_until_a_frame_is_written() {
        let (out, mut inbound) = seat();
        assert!(out.send(vec![b'a'; OUTBOUND_BYTE_CAP + 8]));
        assert!(
            !out.send(b"n".to_vec()),
            "a second frame was queued past the byte cap"
        );
        let taken = inbound.rx.recv().await.expect("frame");
        let Message::Frame(bytes) = taken else {
            panic!("expected a frame");
        };
        assert_eq!(bytes.len(), OUTBOUND_BYTE_CAP + 8);
        assert!(
            !out.send(b"n".to_vec()),
            "a taken frame was released before it was written"
        );
        release(&inbound.queued_bytes, bytes.len());
        assert!(out.send(b"n".to_vec()));
    }
}
