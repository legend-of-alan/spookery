//! Action state: how many times an actor has acted, and the action they are still on.

use serde::{Deserialize, Serialize};

use crate::intent::Target;
use crate::state::change::{self, Change};
use crate::state::component::{Component, Mut, Ref};
use crate::state::id::ActorId;
use crate::state::{entity, Writer};
use crate::Reason;

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

impl Component for Act {
    type Value = Act;
}

impl Act {
    /// The tick the actor is busy until, as a change value.
    fn busy_value(&self) -> change::Value {
        self.busy_until.map_or(change::Value::Empty, count_value)
    }

    /// Busy means an action under way, or a cooldown whose `busy_until` is still after this
    /// tick. An action stays under way until the round that reaches its finish ends.
    pub fn is_busy(&self, world_time: u64) -> bool {
        self.pending.is_some() || self.busy_until.is_some_and(|until| until > world_time)
    }
}

impl<'a> Ref<'a, entity::Actors, Act> {
    /// The actor's action state.
    pub fn get(&self) -> Option<&'a Act> {
        self.entities.components.acts.get(self.id)
    }
}

impl Mut<'_, entity::Actors, Act> {
    /// The one write to the actor's action state: `edit` made to what they hold, reporting
    /// `key`'s change as `read` reads the action state before and after.
    fn write(
        &mut self,
        key: change::Key,
        read: impl Fn(&Act) -> change::Value,
        edit: impl FnOnce(&mut Act),
    ) -> Result<(), Reason> {
        self.check_exists()?;
        let held = self.writer.acts.get(self.id);
        let before = held.map_or(change::Value::Empty, &read);
        let mut next = held.cloned().unwrap_or_default();
        edit(&mut next);
        let change = Change::new(key, before, read(&next));
        let acts = &mut self.writer.components.acts;
        acts.set(self.writer.transaction, self.id, next, [change.into()])
    }

    /// Keep the actor busy until `until`, on `pending` while an action is under way and on
    /// `None` through a cooldown.
    pub fn set_busy(&mut self, until: u64, pending: Option<PendingAction>) -> Result<(), Reason> {
        let key = change::Key::ActorBusy(self.id.to_string());
        self.write(key, Act::busy_value, |act| {
            (act.busy_until, act.pending) = (Some(until), pending);
        })
    }

    /// End the actor's action under way, if any, and free them. The action, so `cancel` and
    /// `finish` see it.
    pub fn take_pending(&mut self) -> Result<Option<PendingAction>, Reason> {
        let (key, mut taken) = (change::Key::ActorBusy(self.id.to_string()), None);
        self.write(key, Act::busy_value, |act| {
            taken = act.pending.take();
            act.busy_until = None;
        })?;
        Ok(taken)
    }

    /// Count one act for the actor in round `round_index`, in the act window the table is in:
    /// the idle one when no player is seated. Only the keys whose values move are reported.
    pub fn count(&mut self, round_index: u64, players: u32) -> Result<(), Reason> {
        let key = change::Key::LastActRound(self.id.to_string());
        let read = |act: &Act| count_value(act.last_act_round);
        self.write(key, read, |act| act.last_act_round = round_index)?;
        let counter = match players {
            0 => ActCounter::IdleActsThisWindow,
            _ => ActCounter::LiveActsThisWindow,
        };
        let read = |act: &Act| count_value(counter.count(act).into());
        self.write(counter.key(self.id.to_string()), read, |act| {
            let count = counter.count_mut(act);
            *count = count.saturating_add(1);
        })
    }
}

impl Writer<'_, entity::Actors> {
    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let actors: Vec<ActorId> = self.kind.keys().cloned().collect();
        for actor in &actors {
            let read = |act: &Act| count_value(counter.count(act).into());
            let key = counter.key(actor.to_string());
            let mut action_state = self.get_mut::<Act>(actor);
            action_state.write(key, read, |act| *counter.count_mut(act) = 0)?;
        }
        Ok(())
    }
}

/// A count as a change value.
fn count_value(count: u64) -> change::Value {
    change::Value::Number(i64::try_from(count).unwrap_or(i64::MAX))
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

    /// This count on `act`.
    pub(crate) fn count(self, act: &Act) -> u32 {
        match self {
            ActCounter::IdleActsThisWindow => act.idle_acts_this_window,
            ActCounter::LiveActsThisWindow => act.live_acts_this_window,
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
