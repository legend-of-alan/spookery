//! The staged save a round writes before commit.

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::Budget;
use crate::canon::Canon;
use crate::model::key::{Change, StateKey, StateValue};
use crate::model::patch::Patch;
use crate::model::{ActCounter, MemoryLine, PendingAction, Presence, State};
use crate::rules::effect::Holder;
use crate::world::World;
use crate::Reason;

pub(crate) struct Draft<'p> {
    canon: Arc<Canon>,
    pub(crate) state: State,
    /// Who is seated for this round. The round reads it and never writes it.
    pub(crate) presence: &'p Presence,
    /// The keys counted toward the budget this round.
    keys: BTreeSet<StateKey>,
    /// The same keys in the order they were counted, so a savepoint can give them back.
    key_order: Vec<StateKey>,
    /// The patch that puts back each write, oldest first. A rollback swaps back every one
    /// since its savepoint, newest first.
    undo: Vec<Patch>,
    /// Each key a write moved, with its value before and after that write, oldest first.
    changes: Vec<Change>,
    /// How many changes the last take had reported.
    taken: usize,
    pub(crate) memories: Vec<MemoryLine>,
    memory_writes: u32,
    _nosend: PhantomData<Rc<()>>,
}

impl<'p> Draft<'p> {
    /// Stage a round on a copy of the world's save, with `presence` seated.
    pub(crate) fn begin<L>(world: &World<L>, presence: &'p Presence) -> Self {
        Self {
            canon: Arc::clone(&world.canon),
            state: world.state.clone(),
            presence,
            keys: BTreeSet::new(),
            key_order: Vec::new(),
            undo: Vec::new(),
            changes: Vec::new(),
            taken: 0,
            memories: Vec::new(),
            memory_writes: 0,
            _nosend: PhantomData,
        }
    }

    pub(crate) fn canon(&self) -> &Canon {
        &self.canon
    }

    pub(crate) fn shared_canon(&self) -> Arc<Canon> {
        Arc::clone(&self.canon)
    }

    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    pub(crate) fn budget(&self) -> &Budget {
        &self.canon.budget
    }

    pub(crate) fn stamp_round(&mut self, round_index: u64, world_time: u64, last_round_at: &str) {
        self.state.round_index = round_index;
        self.state.world_time = world_time;
        self.state.last_round_at = last_round_at.to_string();
    }

    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for actor in ids {
            self.apply(Patch::Acts {
                actor,
                counter,
                count: 0,
            })?;
        }
        Ok(())
    }

    /// Expire every modifier whose tick has come by `world_time`, all its stacks at once.
    pub(crate) fn expire_modifiers(&mut self, world_time: u64) -> Result<(), Reason> {
        let due: Vec<(String, String)> = self
            .state
            .actors
            .iter()
            .flat_map(|(actor, live)| {
                live.modifiers
                    .iter()
                    .filter(|(_, held)| held.until <= world_time)
                    .map(|(id, _)| (actor.clone(), id.clone()))
            })
            .collect();
        for (actor, id) in due {
            self.apply(Patch::Modifier {
                actor,
                id,
                held: None,
            })?;
        }
        Ok(())
    }

    /// Count one act for `actor_id` this round, in the act window the table is in.
    pub(crate) fn count_act(&mut self, actor_id: &str) -> Result<(), Reason> {
        let round = self.state.round_index;
        let act = &self
            .state
            .actors
            .get(actor_id)
            .ok_or(Reason::UnknownId)?
            .act;
        let (counter, count) = if self.presence.players == 0 {
            (ActCounter::IdleActsThisWindow, act.idle_acts_this_window)
        } else {
            (ActCounter::LiveActsThisWindow, act.live_acts_this_window)
        };
        let actor = actor_id.to_string();
        self.apply(Patch::LastActRound {
            actor: actor.clone(),
            round,
        })?;
        self.apply(Patch::Acts {
            actor,
            counter,
            count: count.saturating_add(1),
        })
    }

    /// Keep actor `id` busy until `until`, on `pending` when it is an action under way and
    /// `None` when it is a cooldown.
    pub(crate) fn set_busy(
        &mut self,
        id: &str,
        until: u64,
        pending: Option<PendingAction>,
    ) -> Result<(), Reason> {
        self.apply(Patch::Busy {
            actor: id.to_string(),
            until: Some(until),
            pending,
        })
    }

    /// End actor `id`'s action under way, if any, and free them: it never finishes here.
    pub(crate) fn take_pending(&mut self, id: &str) -> Result<Option<PendingAction>, Reason> {
        let actor = self.state.actors.get(id).ok_or(Reason::UnknownId)?;
        let taken = actor.act.pending.clone();
        self.apply(Patch::Busy {
            actor: id.to_string(),
            until: None,
            pending: None,
        })?;
        Ok(taken)
    }

    /// Write `patch` into the save: the one way a round changes it, so a later read, in the
    /// same effect or the next, sees it. The patch that puts the old value back goes on the
    /// undo log for a savepoint. When the stored value moved, its key is counted toward the
    /// budget and its values before and after are kept for the next take; a value already in
    /// place counts nothing. A modifier's stacks add to the meter its def names: when they
    /// move, that meter's key is counted first and its values kept too, and a modifier put in
    /// place must leave the meter readable before its own key is counted. A write the budget
    /// refuses stays in place for the caller's savepoint to undo.
    pub(crate) fn apply(&mut self, patch: Patch) -> Result<(), Reason> {
        let meter = self.modified_meter(&patch);
        let undo = patch.clone().swap(&mut self.state)?;
        let stacks_moved = undo.stacks() != patch.stacks();
        let own = if undo == patch {
            None
        } else {
            let read = |side: &Patch| side.read(&self.canon, &self.state);
            let values = read(&undo).zip(read(&patch));
            self.undo.push(undo);
            values
        };
        if let Some((actor, meter, before)) = meter {
            let key = StateKey::ActorMeter(actor.clone(), meter.clone());
            if stacks_moved {
                self.count_key(&key)?;
            }
            let after = self
                .state
                .meter_value(&self.canon, &Holder::Actor(actor), &meter);
            if matches!(patch, Patch::Modifier { held: Some(_), .. }) {
                after?;
            }
            if stacks_moved {
                let after = after.map_or(StateValue::Empty, StateValue::Number);
                self.changes.push(Change { key, before, after });
            }
        }
        if let Some(((key, before), (_, after))) = own {
            self.count_key(&key)?;
            self.changes.push(Change { key, before, after });
        }
        Ok(())
    }

    /// For a modifier's patch, the actor whose meter it adds to, the meter its def names, and
    /// that meter's value now.
    fn modified_meter(&self, patch: &Patch) -> Option<(String, String, StateValue)> {
        let Patch::Modifier { actor, id, .. } = patch else {
            return None;
        };
        let meter = self.canon.modifiers.get(id)?.meter.clone();
        let before = self.meter_value(&Holder::Actor(actor.clone()), &meter);
        Some((actor.clone(), meter, before))
    }

    /// Count `key` toward the budget the first time this round, whether or not a write
    /// changed it: a cost of 0 still takes a key.
    pub(crate) fn count_key(&mut self, key: &StateKey) -> Result<(), Reason> {
        if self.keys.contains(key) {
            return Ok(());
        }
        if self.keys.len() >= self.canon.budget.max_state_keys_mutated as usize {
            return Err(Reason::Budget);
        }
        self.keys.insert(key.clone());
        self.key_order.push(key.clone());
        Ok(())
    }

    /// How many keys the round has counted so far.
    #[cfg(test)]
    pub(crate) fn keys_spent(&self) -> usize {
        self.key_order.len()
    }

    /// Every key whose value differs from what it was at the last take, sorted: its value
    /// before its first change since then, and after its last.
    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        let since = std::mem::replace(&mut self.taken, self.changes.len());
        let mut net: BTreeMap<&StateKey, Change> = BTreeMap::new();
        for change in &self.changes[since..] {
            net.entry(&change.key)
                .and_modify(|first| first.after = change.after.clone())
                .or_insert_with(|| change.clone());
        }
        net.into_values()
            .filter(|change| change.before != change.after)
            .collect()
    }

    fn meter_value(&self, holder: &Holder, key: &str) -> StateValue {
        match self.state.meter_value(&self.canon, holder, key) {
            Ok(current) => StateValue::Number(current),
            Err(_) => StateValue::Empty,
        }
    }

    pub(crate) fn count_memory_write(&mut self) -> Result<(), Reason> {
        if u64::from(self.memory_writes) >= self.canon.budget.max_memory_writes {
            return Err(Reason::Budget);
        }
        self.memory_writes += 1;
        Ok(())
    }

    pub(crate) fn push_memory(&mut self, line: MemoryLine) {
        self.memories.push(line);
    }

    /// Where the round has come to, to roll back to. It copies nothing.
    pub(crate) fn savepoint(&self) -> Savepoint {
        Savepoint {
            undo: self.undo.len(),
            keys: self.key_order.len(),
            changes: self.changes.len(),
            taken: self.taken,
            memories: self.memories.len(),
            memory_writes: self.memory_writes,
        }
    }

    /// Undo everything since `point`: swap back each write since, newest first, and give back
    /// the keys, changes and memories added since.
    pub(crate) fn rollback(&mut self, point: Savepoint) {
        for undo in self.undo.drain(point.undo..).rev() {
            // Its field was there when the write landed, and every later write is undone
            // first, so putting it back is never refused.
            let restored = undo.swap(&mut self.state);
            debug_assert!(restored.is_ok(), "an undo was refused: {restored:?}");
        }
        for key in self.key_order.drain(point.keys..) {
            self.keys.remove(&key);
        }
        self.changes.truncate(point.changes);
        self.taken = point.taken;
        self.memories.truncate(point.memories);
        self.memory_writes = point.memory_writes;
    }
}

/// How far the round had come: the length of each log a rollback cuts back.
#[derive(Clone, Copy)]
pub(crate) struct Savepoint {
    undo: usize,
    keys: usize,
    changes: usize,
    taken: usize,
    memories: usize,
    memory_writes: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::Intent;
    use crate::model::meter::MeterWrite;
    use crate::rules::effect::{Effect, EffectKind};
    use crate::World;

    #[test]
    fn a_key_counts_toward_the_budget_only_once_a_write_changes_it() {
        let pack =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let opened = World::open(&pack).expect("open");
        let mut canon = opened.canon().clone();
        canon.budget.max_state_keys_mutated = 2;
        let world = World::from_parts(&pack, Arc::new(canon), opened.state().clone());
        let nobody = crate::model::Presence::default();
        let mut draft = Draft::begin(&world, &nobody);
        // Bram's grit sits clamped at its minimum: taking more moves the offset, not the value.
        let meters = &mut draft.state.actor_mut("bram").unwrap().meters;
        meters.get_mut("grit").unwrap().offset = -5;
        let intent = Intent::habit(world.state(), "mira", "leave_door_note");
        let mut write = |actor: &str, key: &str, delta: i64| {
            let kind = EffectKind::Meter {
                holder: Holder::Actor(actor.to_string()),
                key: key.to_string(),
                write: MeterWrite::Offset(delta),
            };
            Effect::new(intent.actor.as_ref(), &intent.id, kind).apply(&mut draft, &intent)
        };
        assert_eq!(write("mira", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("mira", "rest", 1), Ok(()), "the first key");
        assert_eq!(write("bram", "grit", -1), Ok(()), "offset moved: key two");
        assert_eq!(write("mira", "rest", 1), Ok(()), "a counted key is free");
        assert_eq!(write("aldric", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("aldric", "rest", 1), Err(Reason::Budget));
    }
}
