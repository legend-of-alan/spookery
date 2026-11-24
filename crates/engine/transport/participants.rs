//! Who is connected and seated, and each claim with the messages it keeps for a reconnect.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use spookery_protocol::{Seat, ServerMessage};

use crate::{ActorId, Presence};

/// How many messages a claim keeps for a reconnect. Older ones are dropped, and the
/// `claimed` reply says `is_missed_truncated`.
const MISSED_MESSAGE_CAP: usize = 256;

/// One connection's id. Every connection a router sees takes its id from one counter.
#[derive(Clone, Copy, PartialEq)]
pub struct ConnId(u64);

/// The bare number, as a log's `conn_id` field shows it.
impl fmt::Display for ConnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl ConnId {
    /// The next id from `next`, the counter every connection shares.
    pub fn new(next: &AtomicU64) -> Self {
        Self(next.fetch_add(1, Ordering::Relaxed))
    }
}

/// The one owner of who is connected and who is seated. Rounds and readers take
/// `presence()`.
#[derive(Default)]
pub(super) struct Participants {
    pub(super) connections: Vec<Connection>,
    /// The connection that holds the GM seat.
    pub(super) gm: Option<ConnId>,
    /// Each held claim, oldest first. At most three, so a scan finds one.
    pub(super) claims: Vec<Claim>,
}

impl Participants {
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

    pub(super) fn remove(&mut self, conn: ConnId) {
        self.connections.retain(|connection| connection.id != conn);
    }
}

/// The router's entry for one open `ConnId`: which seat it opened.
pub(super) struct Connection {
    pub(super) id: ConnId,
    pub(super) seat: Seat,
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
