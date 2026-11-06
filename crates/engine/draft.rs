//! The staged save a round writes before commit.

use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::Budget;
use crate::canon::Canon;
use crate::model::key::{Change, StateKey, StateValue};
use crate::model::{ActCounter, Flag, Kept, Live, MemoryLine, PendingAction, Presence, State};
use crate::rules::effect::Holder;
use crate::world::World;
use crate::Reason;

pub(crate) struct Draft<'p> {
    canon: Arc<Canon>,
    pub(crate) state: State,
    /// Who is seated for this round. The round reads it and never writes it.
    pub(crate) presence: &'p Presence,
    keys: BTreeSet<StateKey>,
    /// The counted keys in the order they were counted, so a savepoint can give them back.
    key_order: Vec<StateKey>,
    priors: Vec<Prior>,
    /// How many priors the last take reported.
    priors_taken: usize,
    flags: FlagEdits,
    /// Each live part of the save as it was just before a write to it, oldest first. Rolling
    /// back to a savepoint puts back every part kept since, newest first.
    kept: Vec<Kept>,
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
            priors: Vec::new(),
            priors_taken: 0,
            flags: FlagEdits::default(),
            kept: Vec::new(),
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

    /// The state to write `live` in, kept first so a savepoint can put it back.
    pub(crate) fn live_mut(&mut self, live: &Live) -> &mut State {
        self.keep(live);
        &mut self.state
    }

    /// Keep `live` as it is now, before a write to it.
    pub(crate) fn keep(&mut self, live: &Live) {
        self.kept.push(self.state.keep(live));
    }

    pub(crate) fn stamp_round(&mut self, round_index: u64, world_time: u64, last_round_at: &str) {
        self.state.round_index = round_index;
        self.state.world_time = world_time;
        self.state.last_round_at = last_round_at.to_string();
    }

    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            self.mutate(counter.key(id.clone()), |state| {
                Ok(std::mem::take(counter.count_mut(&mut state.actor_mut(&id)?.act)) != 0)
            })?;
        }
        Ok(())
    }

    /// Expire every modifier whose tick has come by `world_time`, all its stacks at once.
    pub(crate) fn expire_modifiers(&mut self, world_time: u64) -> Result<(), Reason> {
        let due: Vec<(String, String, String)> = self
            .state
            .actors
            .iter()
            .flat_map(|(actor, live)| {
                live.modifiers
                    .iter()
                    .filter(|(_, held)| held.until <= world_time)
                    .map(|(id, held)| (actor.clone(), id.clone(), held.meter.clone()))
            })
            .collect();
        for (actor, id, meter) in due {
            self.track(StateKey::ActorMeter(actor.clone(), meter))?;
            self.mutate(
                StateKey::ActorModifier(actor.clone(), id.clone()),
                |state| Ok(state.actor_mut(&actor)?.modifiers.remove(&id).is_some()),
            )?;
        }
        Ok(())
    }

    /// Count one act for `actor_id` this round, in the act window the table is in.
    pub(crate) fn count_act(&mut self, actor_id: &str) -> Result<(), Reason> {
        let (round_index, players) = (self.state.round_index, self.presence.players);
        let (current_idle, current_live, current_last) = {
            let counters = &self
                .state
                .actors
                .get(actor_id)
                .ok_or(Reason::UnknownId)?
                .act;
            (
                counters.idle_acts_this_window,
                counters.live_acts_this_window,
                counters.last_act_round,
            )
        };
        let new_idle = current_idle.saturating_add(1);
        let new_live = current_live.saturating_add(1);
        let mut keys = Vec::new();
        if round_index != current_last {
            keys.push(StateKey::LastActRound(actor_id.to_string()));
        }
        if players == 0 && new_idle != current_idle {
            keys.push(StateKey::IdleActsThisWindow(actor_id.to_string()));
        }
        if players > 0 && new_live != current_live {
            keys.push(StateKey::LiveActsThisWindow(actor_id.to_string()));
        }
        self.track_many(&keys)?;
        let state = self.live_mut(&Live::Actor(actor_id.to_string()));
        let counters = &mut state.actor_mut(actor_id)?.act;
        counters.last_act_round = round_index;
        if players == 0 {
            counters.idle_acts_this_window = new_idle;
        } else {
            counters.live_acts_this_window = new_live;
        }
        Ok(())
    }

    /// Keep actor `id` busy until `until`, on `pending` when it is an action under way and
    /// `None` when it is a cooldown.
    pub(crate) fn set_busy(
        &mut self,
        id: &str,
        until: u64,
        pending: Option<PendingAction>,
    ) -> Result<(), Reason> {
        self.mutate(StateKey::ActorBusy(id.to_string()), |state| {
            let act = &mut state.actor_mut(id)?.act;
            let changed = act.busy_until != Some(until) || act.pending != pending;
            act.busy_until = Some(until);
            act.pending = pending;
            Ok(changed)
        })
    }

    /// End actor `id`'s action under way, if any, and free them: it never finishes here.
    pub(crate) fn take_pending(&mut self, id: &str) -> Result<Option<PendingAction>, Reason> {
        let mut taken = None;
        self.mutate(StateKey::ActorBusy(id.to_string()), |state| {
            let act = &mut state.actor_mut(id)?.act;
            taken = act.pending.take();
            Ok(act.busy_until.take().is_some() || taken.is_some())
        })?;
        Ok(taken)
    }

    /// Runs `write` on the state, tracks `key` only if its value changed, and keeps the
    /// key budget's order: a key that does not change is never counted. `write` returns
    /// whether it changed the stored value; a meter's is its base and offset, which can move
    /// while the clamped value the key reads stays. The key is counted after the write, so an
    /// `Err` from the budget can leave the write in place: the caller's savepoint undoes it.
    /// `write` may change only the live part `key` sits in; a write that reaches another part
    /// keeps it first.
    pub(crate) fn mutate(
        &mut self,
        key: StateKey,
        write: impl FnOnce(&mut State) -> Result<bool, Reason>,
    ) -> Result<(), Reason> {
        let before = self.read_value(&key);
        if write(self.live_mut(&key.live()))? {
            self.track_before(key, before)?;
        }
        Ok(())
    }

    /// Set or clear `flag`. A write that changes it goes on the flag edits, which report it
    /// as a change, and counts its key toward the budget. As with `mutate`, a budget `Err`
    /// leaves the write for the caller's savepoint to undo.
    pub(crate) fn set_flag(&mut self, flag: Flag, value: bool) -> Result<(), Reason> {
        if flag.get(&self.state) == value {
            return Ok(());
        }
        flag.set(self.live_mut(&flag.key.live()), value)?;
        let counted = self.count_key(&flag.key);
        self.flags.edits.push(FlagEdit { flag, was: !value });
        counted
    }

    pub(crate) fn track(&mut self, key: StateKey) -> Result<(), Reason> {
        self.track_many(std::slice::from_ref(&key))
    }

    /// Record `before` as this key's value from before the write that just landed.
    fn track_before(&mut self, key: StateKey, before: StateValue) -> Result<(), Reason> {
        self.count_key(&key)?;
        if !self.has_prior(&key) {
            self.priors.push(Prior { key, before });
        }
        Ok(())
    }

    /// Count each of `keys` and note its value now. Over the budget, the keys this call
    /// counted are given back.
    fn track_many(&mut self, keys: &[StateKey]) -> Result<(), Reason> {
        let counted = self.key_order.len();
        for key in keys {
            if let Err(code) = self.count_key(key) {
                let dropped = self.drop_keys(counted);
                self.priors.retain(|prior| !dropped.contains(&prior.key));
                return Err(code);
            }
            if !self.has_prior(key) {
                let before = self.read_value(key);
                self.priors.push(Prior {
                    key: key.clone(),
                    before,
                });
            }
        }
        Ok(())
    }

    /// Count `key` toward the budget the first time it changes this round.
    fn count_key(&mut self, key: &StateKey) -> Result<(), Reason> {
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

    /// Give back every key counted after the first `from`, and return them.
    fn drop_keys(&mut self, from: usize) -> Vec<StateKey> {
        let dropped: Vec<StateKey> = self.key_order.drain(from..).collect();
        for key in &dropped {
            self.keys.remove(key);
        }
        dropped
    }

    /// Whether `key` has a prior the next take reports.
    fn has_prior(&self, key: &StateKey) -> bool {
        self.priors[self.priors_taken..]
            .iter()
            .any(|prior| &prior.key == key)
    }

    /// Changes since the last take. The cumulative key set used by `changes` stays.
    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        let taken = std::mem::replace(&mut self.priors_taken, self.priors.len());
        let mut records = Vec::new();
        for prior in &self.priors[taken..] {
            let after = self.read_value(&prior.key);
            if after == prior.before {
                continue;
            }
            records.push(Change {
                key: prior.key.clone(),
                before: prior.before.clone(),
                after,
            });
        }
        records.extend(self.flags.take(&self.state));
        records.sort_by(|left, right| left.key.cmp(&right.key));
        records
    }

    fn read_value(&self, key: &StateKey) -> StateValue {
        match key {
            StateKey::ActorPlace(id) => self
                .state
                .actors
                .get(id)
                .map(|actor| StateValue::Text(actor.place.clone()))
                .unwrap_or(StateValue::Empty),
            // Only `set_flag` writes these, and the flag edits report them.
            StateKey::Flag(..) => StateValue::Empty,
            StateKey::StoryFlag(id, flag) => {
                StateValue::Bool(self.state.has_flag(&Holder::Story(id.clone()), flag))
            }
            StateKey::PlaceMeter(id, meter) => self.meter_value(&Holder::Place(id.clone()), meter),
            StateKey::StoryMeter(id, meter) => self.meter_value(&Holder::Story(id.clone()), meter),
            StateKey::ActorMeter(id, meter) => self.meter_value(&Holder::Actor(id.clone()), meter),
            StateKey::ActorModifier(id, key) => self
                .state
                .actors
                .get(id)
                .and_then(|actor| actor.modifiers.get(key))
                .map(|live| {
                    StateValue::Text(format!(
                        "{:?} until {} max_ticks {}",
                        live.stacks, live.until, live.max_ticks
                    ))
                })
                .unwrap_or(StateValue::Empty),
            StateKey::ActorBusy(id) => self
                .state
                .actors
                .get(id)
                .map(|actor| match actor.act.busy_until {
                    Some(until) => StateValue::Number(i64::try_from(until).unwrap_or(i64::MAX)),
                    None => StateValue::Empty,
                })
                .unwrap_or(StateValue::Empty),
            StateKey::ActorKnownFact(id) => self
                .state
                .actors
                .get(id)
                .map(|actor| StateValue::List(actor.known_facts.clone()))
                .unwrap_or(StateValue::Empty),
            StateKey::LastActRound(id) => self.act_number(id, |act| {
                i64::try_from(act.last_act_round).unwrap_or(i64::MAX)
            }),
            StateKey::IdleActsThisWindow(id) => {
                self.act_number(id, |act| i64::from(act.idle_acts_this_window))
            }
            StateKey::LiveActsThisWindow(id) => {
                self.act_number(id, |act| i64::from(act.live_acts_this_window))
            }
            StateKey::ActorItem(id, item) => self
                .state
                .actors
                .get(id)
                .map(|actor| StateValue::Number(i64::from(actor.item_count(item))))
                .unwrap_or(StateValue::Empty),
            StateKey::PlaceItem(place, item) => self
                .state
                .places
                .get(place)
                .map(|place| {
                    StateValue::Number(i64::from(place.items.get(item).copied().unwrap_or(0)))
                })
                .unwrap_or(StateValue::Empty),
            StateKey::ActorWielded(id) => self
                .state
                .actors
                .get(id)
                .map(|actor| match actor.wielded.as_deref() {
                    Some(item) => StateValue::Text(item.to_string()),
                    None => StateValue::Empty,
                })
                .unwrap_or(StateValue::Empty),
            StateKey::Note(id) => {
                if self.state.notes.iter().any(|note| &note.id == id) {
                    StateValue::Bool(true)
                } else {
                    StateValue::Empty
                }
            }
            StateKey::WorldFlag(key) => {
                StateValue::Bool(self.state.flags.iter().any(|flag| flag == key))
            }
            StateKey::WorldMeter(key) => self.meter_value(&Holder::World, key),
        }
    }

    fn meter_value(&self, holder: &Holder, key: &str) -> StateValue {
        match self.state.meter_value(&self.canon, holder, key) {
            Ok(current) => StateValue::Number(current),
            Err(_) => StateValue::Empty,
        }
    }

    fn act_number(
        &self,
        actor_id: &str,
        read: impl FnOnce(&crate::model::Act) -> i64,
    ) -> StateValue {
        self.state
            .actors
            .get(actor_id)
            .map(|actor| StateValue::Number(read(&actor.act)))
            .unwrap_or(StateValue::Empty)
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
            kept: self.kept.len(),
            keys: self.key_order.len(),
            priors: self.priors.len(),
            priors_taken: self.priors_taken,
            flags: self.flags.edits.len(),
            flags_taken: self.flags.taken,
            memories: self.memories.len(),
            memory_writes: self.memory_writes,
        }
    }

    /// Undo everything since `point`: put back each part kept since, newest first, and give
    /// back the keys, priors, flag edits and memories added since.
    pub(crate) fn rollback(&mut self, point: Savepoint) {
        for kept in self.kept.drain(point.kept..).rev() {
            self.state.restore(kept);
        }
        self.drop_keys(point.keys);
        self.priors.truncate(point.priors);
        self.priors_taken = point.priors_taken;
        self.flags.edits.truncate(point.flags);
        self.flags.taken = point.flags_taken;
        self.memories.truncate(point.memories);
        self.memory_writes = point.memory_writes;
    }
}

/// A tracked key and its value from before the round wrote it.
struct Prior {
    key: StateKey,
    before: StateValue,
}

/// Every write that changed a flag `Flag` keeps, in order, with what the flag was before it.
/// `taken` counts the edits already reported.
#[derive(Default)]
struct FlagEdits {
    edits: Vec<FlagEdit>,
    taken: usize,
}

struct FlagEdit {
    flag: Flag,
    was: bool,
}

impl FlagEdits {
    /// Each flag edited since the last take whose value now differs from before its first
    /// edit since then.
    fn take(&mut self, state: &State) -> Vec<Change> {
        let mut seen: Vec<&StateKey> = Vec::new();
        let mut changes = Vec::new();
        for edit in &self.edits[self.taken..] {
            if seen.contains(&&edit.flag.key) {
                continue;
            }
            seen.push(&edit.flag.key);
            let after = edit.flag.get(state);
            if after != edit.was {
                changes.push(Change {
                    key: edit.flag.key.clone(),
                    before: StateValue::Bool(edit.was),
                    after: StateValue::Bool(after),
                });
            }
        }
        self.taken = self.edits.len();
        changes
    }
}

/// How far the round had come: the length of each log a rollback cuts back.
#[derive(Clone, Copy)]
pub(crate) struct Savepoint {
    kept: usize,
    keys: usize,
    priors: usize,
    priors_taken: usize,
    flags: usize,
    flags_taken: usize,
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
