//! The staged save a round writes before commit.

use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::Budget;
use crate::canon::{Canon, OPEN};
use crate::state::change::{self, Change};
use crate::state::id::{ActorId, EntityId};
use crate::state::{ActCounter, MemoryLine, PendingAction, Presence, State};
use crate::world::World;
use crate::Reason;

pub(crate) struct Draft<'p> {
    canon: Arc<Canon>,
    pub(crate) state: State,
    /// Who is seated for this round. The round reads it and never writes it.
    pub(crate) presence: &'p Presence,
    keys: BTreeSet<change::Key>,
    priors: Vec<Prior>,
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
            priors: Vec::new(),
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

    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    pub(crate) fn stamp_round(&mut self, round_index: u64, world_time: u64, last_round_at: &str) {
        self.state.round_index = round_index;
        self.state.world_time = world_time;
        self.state.last_round_at = last_round_at.to_string();
    }

    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let ids: Vec<ActorId> = self.state.actors.keys().cloned().collect();
        for id in ids {
            self.mutate(counter.key(id.to_string()), |state| {
                Ok(std::mem::take(counter.count_mut(&mut state.actor_mut(&id)?.act)) != 0)
            })?;
        }
        Ok(())
    }

    /// Expire every modifier whose tick has come by `world_time`, all its stacks at once.
    pub(crate) fn expire_modifiers(&mut self, world_time: u64) -> Result<(), Reason> {
        let due: Vec<(ActorId, String, String)> = self
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
            self.track(change::Key::ActorMeter(actor.to_string(), meter))?;
            self.mutate(
                change::Key::ActorModifier(actor.to_string(), id.clone()),
                |state| Ok(state.actor_mut(&actor)?.modifiers.remove(&id).is_some()),
            )?;
        }
        Ok(())
    }

    /// Count one act for `actor_id` this round, in the act window the table is in.
    pub(crate) fn count_act(&mut self, actor_id: &ActorId) -> Result<(), Reason> {
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
            keys.push(change::Key::LastActRound(actor_id.to_string()));
        }
        if players == 0 && new_idle != current_idle {
            keys.push(change::Key::IdleActsThisWindow(actor_id.to_string()));
        }
        if players > 0 && new_live != current_live {
            keys.push(change::Key::LiveActsThisWindow(actor_id.to_string()));
        }
        self.track_many(&keys)?;
        let counters = &mut self.state.actor_mut(actor_id)?.act;
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
        id: &ActorId,
        until: u64,
        pending: Option<PendingAction>,
    ) -> Result<(), Reason> {
        self.mutate(change::Key::ActorBusy(id.to_string()), |state| {
            let act = &mut state.actor_mut(id)?.act;
            let changed = act.busy_until != Some(until) || act.pending != pending;
            act.busy_until = Some(until);
            act.pending = pending;
            Ok(changed)
        })
    }

    /// End actor `id`'s action under way, if any, and free them: it never finishes here.
    pub(crate) fn take_pending(&mut self, id: &ActorId) -> Result<Option<PendingAction>, Reason> {
        let mut taken = None;
        self.mutate(change::Key::ActorBusy(id.to_string()), |state| {
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
    pub(crate) fn mutate(
        &mut self,
        key: change::Key,
        write: impl FnOnce(&mut State) -> Result<bool, Reason>,
    ) -> Result<(), Reason> {
        let before = self.read_value(&key);
        if write(&mut self.state)? {
            self.track_before(key, before)?;
        }
        Ok(())
    }

    pub(crate) fn track(&mut self, key: change::Key) -> Result<(), Reason> {
        self.track_many(std::slice::from_ref(&key))
    }

    /// Record `before` as this key's value from before the write that just landed.
    fn track_before(&mut self, key: change::Key, before: change::Value) -> Result<(), Reason> {
        let max = self.canon.budget.max_state_keys_mutated as usize;
        self.reserve_key(&key, &mut Vec::new(), max)?;
        if !self.priors.iter().any(|prior| prior.key == key) {
            self.priors.push(Prior { key, before });
        }
        Ok(())
    }

    fn track_many(&mut self, keys: &[change::Key]) -> Result<(), Reason> {
        let max = self.canon.budget.max_state_keys_mutated as usize;
        let mut added = Vec::new();
        for key in keys {
            self.reserve_key(key, &mut added, max)?;
            if !self.priors.iter().any(|prior| &prior.key == key) {
                let before = self.read_value(key);
                self.priors.push(Prior {
                    key: key.clone(),
                    before,
                });
            }
        }
        Ok(())
    }

    fn reserve_key(
        &mut self,
        key: &change::Key,
        added: &mut Vec<change::Key>,
        max: usize,
    ) -> Result<(), Reason> {
        if self.keys.contains(key) {
            return Ok(());
        }
        if self.keys.len() >= max {
            for key in added {
                self.keys.remove(key);
                self.priors.retain(|prior| &prior.key != key);
            }
            return Err(Reason::Budget);
        }
        self.keys.insert(key.clone());
        added.push(key.clone());
        Ok(())
    }

    /// Changes since the last take. The cumulative key set used by `changes` stays.
    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        let pending = std::mem::take(&mut self.priors);
        let mut records = Vec::new();
        for prior in pending {
            let after = self.read_value(&prior.key);
            if after == prior.before {
                continue;
            }
            records.push(Change {
                key: prior.key,
                before: prior.before,
                after,
            });
        }
        records.sort_by(|left, right| left.key.cmp(&right.key));
        records
    }

    fn read_value(&self, key: &change::Key) -> change::Value {
        match key {
            change::Key::ActorPlace(id) => self
                .state
                .actors
                .get(id.as_str())
                .map(|actor| change::Value::Text(actor.place.clone()))
                .unwrap_or(change::Value::Empty),
            change::Key::ActorFlag(id, flag) => self
                .state
                .actors
                .get(id.as_str())
                .map(|actor| change::Value::Bool(actor.flags.iter().any(|item| item == flag)))
                .unwrap_or(change::Value::Empty),
            change::Key::PlaceFlag(id, flag) => change::Value::Bool(
                self.state
                    .has_flag(&EntityId::Place(id.clone().into()), flag),
            ),
            change::Key::StoryFlag(id, flag) => change::Value::Bool(
                self.state
                    .has_flag(&EntityId::Story(id.clone().into()), flag),
            ),
            change::Key::ExitOpen(place, exit) => {
                let entity = EntityId::Exit(place.clone().into(), exit.clone());
                change::Value::Bool(self.state.has_flag(&entity, OPEN))
            }
            change::Key::PlaceMeter(id, meter) => {
                self.meter_value(&EntityId::Place(id.clone().into()), meter)
            }
            change::Key::StoryMeter(id, meter) => {
                self.meter_value(&EntityId::Story(id.clone().into()), meter)
            }
            change::Key::ActorMeter(id, meter) => {
                self.meter_value(&EntityId::Actor(id.clone().into()), meter)
            }
            change::Key::ActorModifier(id, key) => self
                .state
                .actors
                .get(id.as_str())
                .and_then(|actor| actor.modifiers.get(key))
                .map(|live| {
                    change::Value::Text(format!(
                        "{:?} until {} max_ticks {}",
                        live.stacks, live.until, live.max_ticks
                    ))
                })
                .unwrap_or(change::Value::Empty),
            change::Key::ActorBusy(id) => self
                .state
                .actors
                .get(id.as_str())
                .map(|actor| match actor.act.busy_until {
                    Some(until) => change::Value::Number(i64::try_from(until).unwrap_or(i64::MAX)),
                    None => change::Value::Empty,
                })
                .unwrap_or(change::Value::Empty),
            change::Key::ActorKnownFact(id) => self
                .state
                .actors
                .get(id.as_str())
                .map(|actor| change::Value::List(actor.known_facts.clone()))
                .unwrap_or(change::Value::Empty),
            change::Key::LastActRound(id) => self.act_number(id, |act| {
                i64::try_from(act.last_act_round).unwrap_or(i64::MAX)
            }),
            change::Key::IdleActsThisWindow(id) => {
                self.act_number(id, |act| i64::from(act.idle_acts_this_window))
            }
            change::Key::LiveActsThisWindow(id) => {
                self.act_number(id, |act| i64::from(act.live_acts_this_window))
            }
            change::Key::ActorItem(id, item) => self
                .state
                .actors
                .get(id.as_str())
                .map(|actor| change::Value::Number(i64::from(actor.item_count(item))))
                .unwrap_or(change::Value::Empty),
            change::Key::PlaceItem(place, item) => self
                .state
                .places
                .get(place.as_str())
                .map(|place| {
                    change::Value::Number(i64::from(place.items.get(item).copied().unwrap_or(0)))
                })
                .unwrap_or(change::Value::Empty),
            change::Key::ActorWielded(id) => self
                .state
                .actors
                .get(id.as_str())
                .map(|actor| match actor.wielded.as_deref() {
                    Some(item) => change::Value::Text(item.to_string()),
                    None => change::Value::Empty,
                })
                .unwrap_or(change::Value::Empty),
            change::Key::Note(id) => {
                if self.state.notes.iter().any(|note| &note.id == id) {
                    change::Value::Bool(true)
                } else {
                    change::Value::Empty
                }
            }
            change::Key::WorldFlag(key) => {
                change::Value::Bool(self.state.flags.iter().any(|flag| flag == key))
            }
            change::Key::WorldMeter(key) => self.meter_value(&EntityId::World, key),
        }
    }

    fn meter_value(&self, entity: &EntityId, key: &str) -> change::Value {
        match self.state.meter_value(&self.canon, entity, key) {
            Ok(current) => change::Value::Number(current),
            Err(_) => change::Value::Empty,
        }
    }

    fn act_number(
        &self,
        actor_id: &str,
        read: impl FnOnce(&crate::state::Act) -> i64,
    ) -> change::Value {
        self.state
            .actors
            .get(actor_id)
            .map(|actor| change::Value::Number(read(&actor.act)))
            .unwrap_or(change::Value::Empty)
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

    pub(crate) fn savepoint(&self) -> Savepoint {
        Savepoint {
            state: self.state.clone(),
            keys: self.keys.clone(),
            priors: self.priors.clone(),
            memories: self.memories.clone(),
            memory_writes: self.memory_writes,
        }
    }

    pub(crate) fn rollback(&mut self, point: Savepoint) {
        self.state = point.state;
        self.keys = point.keys;
        self.priors = point.priors;
        self.memories = point.memories;
        self.memory_writes = point.memory_writes;
    }
}

/// A tracked key and its value from before the round wrote it.
#[derive(Clone)]
struct Prior {
    key: change::Key,
    before: change::Value,
}

pub(crate) struct Savepoint {
    state: State,
    keys: BTreeSet<change::Key>,
    priors: Vec<Prior>,
    memories: Vec<MemoryLine>,
    memory_writes: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::Intent;
    use crate::rules::effect::{Effect, EffectKind};
    use crate::state::meter::MeterWrite;
    use crate::World;

    #[test]
    fn a_key_counts_toward_the_budget_only_once_a_write_changes_it() {
        let pack =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let opened = World::open(&pack).expect("open");
        let mut canon = opened.canon().clone();
        canon.budget.max_state_keys_mutated = 2;
        let world = World::from_parts(&pack, Arc::new(canon), opened.state().clone());
        let nobody = crate::state::Presence::default();
        let mut draft = Draft::begin(&world, &nobody);
        // Bram's grit sits clamped at its minimum: taking more moves the offset, not the value.
        let meters = &mut draft.state.actor_mut(&"bram".into()).unwrap().meters;
        meters.get_mut("grit").unwrap().offset = -5;
        let intent = Intent::habit(world.state(), &"mira".into(), "leave_door_note");
        let mut write = |actor: &str, key: &str, delta: i64| {
            let kind = EffectKind::Meter {
                entity: EntityId::Actor(actor.into()),
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
