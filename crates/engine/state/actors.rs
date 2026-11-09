//! The actors' components, one store per component, keyed by actor id.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::change;
use super::grouped;
use super::id::ActorId;
use super::meter::Meter;
use super::modifier::Modifier;
use crate::intent::{Target, Viewer};

/// Every actor's components, by actor id. An actor exists when it has a kind; `kind`, `place`
/// and `acts` have an entry for every actor, the rest only for actors holding something.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActorComponents {
    /// Whether each actor is a PC or an NPC. An actor exists when it has a kind.
    pub kind: BTreeMap<ActorId, ActorKind>,
    /// Where each actor stands: a place id, or `exit:<place>.<exit>` while they walk.
    pub place: BTreeMap<ActorId, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub wielded: BTreeMap<ActorId, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub known_facts: BTreeMap<ActorId, Vec<String>>,
    /// Each place an actor has stood at the end of a round, with the names of the exits they
    /// saw there then.
    #[serde(with = "grouped", skip_serializing_if = "BTreeMap::is_empty")]
    pub visited: BTreeMap<(ActorId, String), Vec<String>>,
    pub acts: BTreeMap<ActorId, Act>,
    #[serde(with = "grouped")]
    pub items: BTreeMap<(ActorId, String), u32>,
    #[serde(with = "grouped")]
    pub meters: BTreeMap<(ActorId, String), Meter>,
    /// The modifiers each actor holds, by modifier id.
    #[serde(with = "grouped")]
    pub modifiers: BTreeMap<(ActorId, String), Modifier>,
    #[serde(with = "grouped::lists")]
    pub flags: BTreeSet<(ActorId, String)>,
}

/// Whether an actor is a PC or an NPC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Npc,
    Pc,
}

impl ActorKind {
    /// The seat actor `id` of this kind acts from.
    pub fn viewer(self, id: &ActorId) -> Viewer {
        match self {
            ActorKind::Pc => Viewer::Player(id.clone()),
            ActorKind::Npc => Viewer::Npc(id.clone()),
        }
    }
}

/// How many times an actor has acted, and the action they are still on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Act {
    pub last_act_round: u64,
    pub idle_acts_this_window: u32,
    pub live_acts_this_window: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingAction>,
}

impl Act {
    /// Busy means an action under way, or a cooldown whose `busy_until` is still after this
    /// tick. An action stays under way until the round that reaches its finish ends.
    pub fn is_busy(&self, world_time: u64) -> bool {
        self.pending.is_some() || self.busy_until.is_some_and(|until| until > world_time)
    }
}

/// One of the `Act` counts the end of its act window sets back to 0.
#[derive(Clone, Copy)]
pub(crate) enum ActCounter {
    IdleActsThisWindow,
    LiveActsThisWindow,
}

impl ActCounter {
    /// The key this count is tracked under for actor `id`.
    pub(crate) fn key(self, id: String) -> change::Key {
        match self {
            ActCounter::IdleActsThisWindow => change::Key::IdleActsThisWindow(id),
            ActCounter::LiveActsThisWindow => change::Key::LiveActsThisWindow(id),
        }
    }

    /// This count on `act`, to write.
    pub(crate) fn count_mut(self, act: &mut Act) -> &mut u32 {
        match self {
            ActCounter::IdleActsThisWindow => &mut act.idle_acts_this_window,
            ActCounter::LiveActsThisWindow => &mut act.live_acts_this_window,
        }
    }
}

/// A timed action that finishes when `busy_until` is reached: its verb, who queued it, the
/// target it was queued against, and the rule it started with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingAction {
    pub verb: String,
    pub source: crate::intent::Source,
    #[serde(default, skip_serializing_if = "Target::is_empty")]
    pub target: Target,
    pub rule: String,
}
