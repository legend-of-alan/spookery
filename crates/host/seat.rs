//! Who is connected and seated, each claim and the messages it keeps for a reconnect, and
//! each connection's capped outbound queue and its writer.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use spookery_engine::{ActorId, Presence};
use tokio::sync::{mpsc, watch};

use super::limits::{MISSED_MESSAGE_CAP, OUTBOUND_CAP_BYTES, OUTBOUND_MESSAGE_CAP};
use crate::protocol::{Seat, ServerMessage};

/// One connection's id, on the game port or the MCP endpoint.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct ConnId(u64);

/// The bare number, as a log's `conn_id` field shows it.
impl fmt::Display for ConnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl ConnId {
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
    pub(super) gm: Option<ConnId>,
    /// Each held claim, oldest first. At most three, so a scan finds one.
    pub(super) claims: Vec<Claim>,
}

impl Seats {
    /// The claimed PCs, sorted, so a round records the same `actors_present` bytes. A claim
    /// whose connection dropped is still present.
    pub(super) fn presence(&self) -> Presence {
        let mut actors_present: Vec<ActorId> = self
            .claims
            .iter()
            .map(|claim| claim.actor_id.clone())
            .collect();
        actors_present.sort();
        Presence {
            players: u32::try_from(actors_present.len()).unwrap_or(u32::MAX),
            actors_present,
        }
    }

    /// The claim `conn` drives.
    pub(super) fn claim_of(&self, conn: ConnId) -> Option<&Claim> {
        self.claims.iter().find(|claim| claim.conn == Some(conn))
    }

    pub(super) fn claim_of_mut(&mut self, conn: ConnId) -> Option<&mut Claim> {
        self.claims
            .iter_mut()
            .find(|claim| claim.conn == Some(conn))
    }

    /// The claim on the PC `actor_id`.
    pub(super) fn claim_of_actor_mut(&mut self, actor_id: &ActorId) -> Option<&mut Claim> {
        self.claims
            .iter_mut()
            .find(|claim| claim.actor_id == *actor_id)
    }

    pub(super) fn connection(&self, conn: ConnId) -> Option<&Connection> {
        self.connections
            .iter()
            .find(|connection| connection.id == conn)
    }

    /// Drops the connection. Dropping it is its cancel.
    pub(super) fn take(&mut self, conn: ConnId) -> Option<Connection> {
        let index = self
            .connections
            .iter()
            .position(|connection| connection.id == conn)?;
        Some(self.connections.remove(index))
    }
}

/// The host's entry for one open `ConnId`: which seat it opened and its outbound queue.
pub(super) struct Connection {
    pub(super) id: ConnId,
    pub(super) seat: Seat,
    pub(super) out: MessageSender,
}

/// One player's hold on a PC. It outlives its connection and ends only on `release`. A
/// claim by its `token` takes it back, with the messages it kept.
pub(super) struct Claim {
    pub(super) token: String,
    pub(super) actor_id: ActorId,
    /// The connection that drives it, while one does.
    pub(super) conn: Option<ConnId>,
    /// The newest `MISSED_MESSAGE_CAP` messages sent to this claim, oldest first.
    kept: VecDeque<ServerMessage>,
    /// The `seq` of the newest message dropped past `MISSED_MESSAGE_CAP`, or 0.
    last_lost_seq: u64,
}

impl Claim {
    pub(super) fn new(token: String, actor_id: ActorId) -> Self {
        Self {
            token,
            actor_id,
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
        let missed_messages = self
            .kept
            .iter()
            .filter(|message| message.seq.is_some_and(|seq| seq > last_seq))
            .cloned()
            .collect();
        (missed_messages, last_seq < self.last_lost_seq)
    }
}

/// The host's end of one seat's message queue. The host sends that seat messages with it.
/// Also holds the seat's cancel: dropping it cancels the seat.
pub(super) struct MessageSender {
    tx: mpsc::Sender<Vec<u8>>,
    queued_bytes: Arc<AtomicUsize>,
    _cancel: watch::Sender<()>,
}

/// The writer's end of one seat's message queue. That seat's socket or event writer reads from it.
/// A message's bytes stay counted until it is written.
pub(super) struct MessageReceiver {
    rx: mpsc::Receiver<Vec<u8>>,
    queued_bytes: Arc<AtomicUsize>,
    pub(super) cancel: watch::Receiver<()>,
}

impl MessageSender {
    /// Queues one message. False when the queue is full or closed.
    pub(super) fn send(&self, message: Vec<u8>) -> bool {
        let len = message.len();
        if !self.reserve(len) {
            return false;
        }
        if self.tx.try_send(message).is_err() {
            release(&self.queued_bytes, len);
            return false;
        }
        true
    }

    /// An empty queue takes one message of any size, so a welcome still fits.
    fn reserve(&self, len: usize) -> bool {
        self.queued_bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
                if cur == 0 {
                    return Some(len);
                }
                let next = cur.saturating_add(len);
                if next > OUTBOUND_CAP_BYTES {
                    None
                } else {
                    Some(next)
                }
            })
            .is_ok()
    }
}

impl MessageReceiver {
    /// The next message, or `None` once the queue is closed and empty. Its bytes are released
    /// as it is handed on, for a writer that cannot see the send.
    pub(super) fn poll_message(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<u8>>> {
        let message = ready!(self.rx.poll_recv(cx));
        if let Some(bytes) = &message {
            release(&self.queued_bytes, bytes.len());
        }
        Poll::Ready(message)
    }
}

fn release(queued: &AtomicUsize, len: usize) {
    queued.update(Ordering::AcqRel, Ordering::Acquire, |cur| {
        cur.saturating_sub(len)
    });
}

/// One seat's message queue. Like `mpsc::channel()`, it returns the sender and
/// receiver halves. A message is counted against `OUTBOUND_MESSAGE_CAP` and
/// `OUTBOUND_CAP_BYTES` from the moment it is queued until the writer has sent
/// it.
pub(super) fn open_queue() -> (MessageSender, MessageReceiver) {
    let (tx, rx) = mpsc::channel(OUTBOUND_MESSAGE_CAP);
    let (cancel_tx, cancel_rx) = watch::channel(());
    let queued_bytes = Arc::new(AtomicUsize::new(0));
    (
        MessageSender {
            tx,
            queued_bytes: Arc::clone(&queued_bytes),
            _cancel: cancel_tx,
        },
        MessageReceiver {
            rx,
            queued_bytes,
            cancel: cancel_rx,
        },
    )
}

/// Sends each queued message to `sink` until the queue closes, the seat is
/// cancelled, or `sink` fails. A message's bytes are released once it is sent.
pub(super) async fn write_messages(
    mut sink: impl AsyncFnMut(&[u8]) -> bool,
    mut messages_rx: MessageReceiver,
) {
    loop {
        let message = tokio::select! {
            biased;
            message = messages_rx.rx.recv() => message,
            _ = messages_rx.cancel.changed() => break,
        };
        let Some(bytes) = message else {
            break;
        };
        tokio::select! {
            biased;
            sent = sink(&bytes) => {
                if !sent {
                    break;
                }
                release(&messages_rx.queued_bytes, bytes.len());
            }
            _ = messages_rx.cancel.changed() => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_cap_rejects_before_the_byte_cap() {
        let (messages_tx, _messages_rx) = open_queue();
        for _ in 0..OUTBOUND_MESSAGE_CAP {
            assert!(messages_tx.send(b"n".to_vec()));
        }
        assert!(!messages_tx.send(b"n".to_vec()));
    }

    #[tokio::test]
    async fn the_byte_cap_rejects_until_a_message_is_written() {
        let (messages_tx, mut messages_rx) = open_queue();
        assert!(messages_tx.send(vec![b'a'; OUTBOUND_CAP_BYTES + 8]));
        assert!(
            !messages_tx.send(b"n".to_vec()),
            "a second message was queued past the byte cap"
        );
        let bytes = messages_rx.rx.recv().await.expect("message");
        assert_eq!(bytes.len(), OUTBOUND_CAP_BYTES + 8);
        assert!(
            !messages_tx.send(b"n".to_vec()),
            "a taken message was released before it was written"
        );
        release(&messages_rx.queued_bytes, bytes.len());
        assert!(messages_tx.send(b"n".to_vec()));
    }
}
