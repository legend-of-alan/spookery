//! Action state: how many times an actor has acted, and the action they are still on.

use serde::{Deserialize, Serialize};
use spookery_protocol::ChangeValue;

use crate::ecs::change::{self, Change};
use crate::ecs::component::{Component, Mut, Ref};
use crate::ecs::id::ActorId;
use crate::ecs::prelude::*;
use crate::ecs::{entity, Reader, Writer};
use crate::intent::Target;
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

/// Its change keys name what moved instead: `busy`, `last_act_round` or an act window's count,
/// such as `actor_busy rowan`.
impl Component for Act {
    type Value = Act;
    const ID: &'static str = "act";
}

/// The names `Act`'s change keys spell.
impl Act {
    /// `busy_until` and `pending`.
    pub(crate) const BUSY: &'static str = "busy";
    const LAST_ACT_ROUND: &'static str = "last_act_round";
    const IDLE_ACTS_THIS_WINDOW: &'static str = "idle_acts_this_window";
    const LIVE_ACTS_THIS_WINDOW: &'static str = "live_acts_this_window";
}

impl<'a> Ref<'a, entity::Actors, Act> {
    /// The actor's action state.
    pub fn get(&self) -> Option<&'a Act> {
        self.store().get(self.id)
    }

    /// Busy means an action under way, or a cooldown whose `busy_until` is still after
    /// `world_time`. An action stays under way until the round that reaches its finish ends.
    pub fn is_busy(&self, world_time: u64) -> bool {
        self.get().is_some_and(|act| {
            act.pending.is_some() || act.busy_until.is_some_and(|until| until > world_time)
        })
    }
}

impl Mut<'_, entity::Actors, Act> {
    /// The one write to the actor's action state: `edit` made to what they hold, reporting the
    /// change under `component_id` as `read` reads the action state before and after.
    fn write(
        &mut self,
        component_id: &'static str,
        read: impl Fn(&Act) -> Option<ChangeValue>,
        edit: impl FnOnce(&mut Act),
    ) -> Result<(), Reason> {
        self.check_exists()?;
        let held = self.store().get(self.id);
        let before = held.and_then(&read);
        let mut next = held.cloned().unwrap_or_default();
        edit(&mut next);
        let key = change::Key::new(component_id, self.id.as_id_ref(), None);
        let change = Change::new(key, before, read(&next));
        let (id, (acts, transaction)) = (self.id, self.store_mut());
        acts.set(transaction, id, next, [change.into()])
    }

    /// Keep the actor busy until `until`, on `pending` while an action is under way and on
    /// `None` through a cooldown.
    pub fn set_busy(&mut self, until: u64, pending: Option<PendingAction>) -> Result<(), Reason> {
        self.write(Act::BUSY, busy_value, |act| {
            (act.busy_until, act.pending) = (Some(until), pending);
        })
    }

    /// End the actor's action under way, if any, and free them. The action, so `cancel` and
    /// `finish` see it.
    pub fn take_pending(&mut self) -> Result<Option<PendingAction>, Reason> {
        let mut taken = None;
        self.write(Act::BUSY, busy_value, |act| {
            taken = act.pending.take();
            act.busy_until = None;
        })?;
        Ok(taken)
    }

    /// Count one act for the actor in round `round_index`, in the act window the table is in:
    /// the idle one when no player is seated. Only the keys whose values move are reported.
    pub fn count(&mut self, round_index: u64, players: u32) -> Result<(), Reason> {
        let read = |act: &Act| Some(count_value(act.last_act_round));
        self.write(Act::LAST_ACT_ROUND, read, |act| {
            act.last_act_round = round_index
        })?;
        let counter = match players {
            0 => ActCounter::IdleActsThisWindow,
            _ => ActCounter::LiveActsThisWindow,
        };
        let read = |act: &Act| Some(count_value(counter.get(act).into()));
        self.write(counter.id(), read, |act| {
            let count = counter.get_mut(act);
            *count = count.saturating_add(1);
        })
    }
}

impl Writer<'_, entity::Actors> {
    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let actors = Reader::new(&*self.components, self.canon).query::<ActorId>();
        let actors: Vec<ActorId> = actors.iter().cloned().collect();
        for actor in &actors {
            let read = |act: &Act| Some(count_value(counter.get(act).into()));
            let mut action_state = self.get_mut::<Act>(actor);
            action_state.write(counter.id(), read, |act| *counter.get_mut(act) = 0)?;
        }
        Ok(())
    }
}

/// The tick the actor is busy until, as a change value.
fn busy_value(act: &Act) -> Option<ChangeValue> {
    act.busy_until.map(count_value)
}

/// A count as a change value.
fn count_value(count: u64) -> ChangeValue {
    ChangeValue::Number(count.try_into().unwrap_or(i64::MAX))
}

/// One of the `Act` counts the end of its act window sets back to 0.
#[derive(Clone, Copy)]
pub(crate) enum ActCounter {
    IdleActsThisWindow,
    LiveActsThisWindow,
}

impl ActCounter {
    /// The id this count's change keys spell.
    fn id(self) -> &'static str {
        match self {
            ActCounter::IdleActsThisWindow => Act::IDLE_ACTS_THIS_WINDOW,
            ActCounter::LiveActsThisWindow => Act::LIVE_ACTS_THIS_WINDOW,
        }
    }

    /// This count on `act`.
    pub(crate) fn get(self, act: &Act) -> u32 {
        match self {
            ActCounter::IdleActsThisWindow => act.idle_acts_this_window,
            ActCounter::LiveActsThisWindow => act.live_acts_this_window,
        }
    }

    /// This count on `act`, to write.
    pub(crate) fn get_mut(self, act: &mut Act) -> &mut u32 {
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
