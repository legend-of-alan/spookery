//! The staged save a round writes before commit.

use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::Canon;
use crate::canon::{Budget, PausePolicy};
use crate::model::key::{Change, StateKey, StateValue};
use crate::model::{ActCounter, MemoryLine, Presence, RunState, State};
use crate::world::World;
use crate::Reason;

pub(crate) struct Draft<'p> {
    canon: Arc<Canon>,
    pub(crate) state: State,
    /// Who is seated for this round. The round reads it and never writes it.
    pub(crate) presence: &'p Presence,
    keys: BTreeSet<StateKey>,
    priors: Vec<PriorValue>,
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

    pub(crate) fn apply_pause_policy(&mut self) -> Result<(), Reason> {
        let players = self.presence.players;
        let policies = self.canon.budget.pause;
        let clock_ids: Vec<String> = self.state.clocks.keys().cloned().collect();
        for clock_id in clock_ids {
            let Some(clock) = self.state.clocks.get(&clock_id) else {
                continue;
            };
            if !clock.requires_presence {
                continue;
            }
            let paused = players == 0 && policies.presence_clocks == PausePolicy::WhenEmpty;
            if players > 0 {
                self.set_clock_paused(&clock_id, false)?;
            } else if paused {
                self.set_clock_paused(&clock_id, true)?;
            }
        }
        self.state.pause.presence_clocks = match policies.presence_clocks {
            PausePolicy::WhenEmpty if players == 0 => RunState::Paused,
            PausePolicy::WhenEmpty | PausePolicy::AlwaysRun => RunState::Running,
        };
        Ok(())
    }

    fn set_clock_paused(&mut self, clock_id: &str, paused: bool) -> Result<(), Reason> {
        self.mutate(StateKey::ClockPaused(clock_id.to_string()), |state| {
            let clock = state.clocks.get_mut(clock_id).ok_or(Reason::UnknownId)?;
            Ok(std::mem::replace(&mut clock.paused, paused) != paused)
        })
    }

    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            self.mutate(counter.key(id.clone()), |state| {
                Ok(std::mem::take(counter.count_mut(state.actor_mut(&id)?.act_mut())) != 0)
            })?;
        }
        Ok(())
    }

    pub(crate) fn count_npc_act(&mut self, actor_id: &str, players: u32) -> Result<(), Reason> {
        let round_index = self.state.round_index;
        let (current_idle, current_live, current_last) = {
            let counters = self
                .state
                .actors
                .get(actor_id)
                .ok_or(Reason::UnknownId)?
                .act();
            (
                counters.idle_acts_today,
                counters.live_acts_this_hour,
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
            keys.push(StateKey::IdleActsToday(actor_id.to_string()));
        }
        if players > 0 && new_live != current_live {
            keys.push(StateKey::LiveActsThisHour(actor_id.to_string()));
        }
        self.track_many(&keys)?;
        let counters = self.state.actor_mut(actor_id)?.act_mut();
        counters.last_act_round = round_index;
        if players == 0 {
            counters.idle_acts_today = new_idle;
        } else {
            counters.live_acts_this_hour = new_live;
        }
        Ok(())
    }

    /// Runs `write` on the state, tracks `key` only if its value changed, and keeps the
    /// key budget's order: a key that does not change is never counted. `write` returns
    /// whether it changed the stored value; a meter's is its base and offset, which can move
    /// while the clamped value the key reads stays. The key is counted after the write, so an
    /// `Err` from the budget can leave the write in place: the caller's savepoint undoes it.
    pub(crate) fn mutate(
        &mut self,
        key: StateKey,
        write: impl FnOnce(&mut State) -> Result<bool, Reason>,
    ) -> Result<(), Reason> {
        let before = self.read_value(&key);
        if write(&mut self.state)? {
            self.track_before(key, before)?;
        }
        Ok(())
    }

    pub(crate) fn track(&mut self, key: StateKey) -> Result<(), Reason> {
        self.track_many(std::slice::from_ref(&key))
    }

    /// Record `before` as this key's value from before the write that just landed.
    fn track_before(&mut self, key: StateKey, before: StateValue) -> Result<(), Reason> {
        let max = self.canon.budget.max_state_keys_mutated as usize;
        self.admit(&key, &mut Vec::new(), max)?;
        if !self.priors.iter().any(|prior| prior.key == key) {
            self.priors.push(PriorValue { key, before });
        }
        Ok(())
    }

    fn track_many(&mut self, keys: &[StateKey]) -> Result<(), Reason> {
        let max = self.canon.budget.max_state_keys_mutated as usize;
        let mut added = Vec::new();
        for key in keys {
            self.admit(key, &mut added, max)?;
            if !self.priors.iter().any(|prior| &prior.key == key) {
                let before = self.read_value(key);
                self.priors.push(PriorValue {
                    key: key.clone(),
                    before,
                });
            }
        }
        Ok(())
    }

    fn admit(
        &mut self,
        key: &StateKey,
        added: &mut Vec<StateKey>,
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

    fn read_value(&self, key: &StateKey) -> StateValue {
        match key {
            StateKey::ActorPlace(id) => self
                .state
                .actors
                .get(id)
                .map(|actor| StateValue::Text(actor.place().to_string()))
                .unwrap_or(StateValue::Empty),
            StateKey::ActorFlag(id, flag) => self
                .state
                .actors
                .get(id)
                .map(|actor| StateValue::Bool(actor.flags().iter().any(|item| item == flag)))
                .unwrap_or(StateValue::Empty),
            StateKey::ActorMeter(id, meter) => self.meter_value(id, meter),
            StateKey::ActorBusy(id) => self
                .state
                .actors
                .get(id)
                .map(|actor| match actor.act().busy_until {
                    Some(until) => StateValue::Number(i64::try_from(until).unwrap_or(i64::MAX)),
                    None => StateValue::Empty,
                })
                .unwrap_or(StateValue::Empty),
            StateKey::ActorKnownFact(id) => self
                .state
                .actors
                .get(id)
                .map(|actor| StateValue::List(actor.known_facts().to_vec()))
                .unwrap_or(StateValue::Empty),
            StateKey::LastActRound(id) => self.act_number(id, |act| {
                i64::try_from(act.last_act_round).unwrap_or(i64::MAX)
            }),
            StateKey::IdleActsToday(id) => {
                self.act_number(id, |act| i64::from(act.idle_acts_today))
            }
            StateKey::LiveActsThisHour(id) => {
                self.act_number(id, |act| i64::from(act.live_acts_this_hour))
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
                .map(|actor| match actor.wielded() {
                    Some(item) => StateValue::Text(item.to_string()),
                    None => StateValue::Empty,
                })
                .unwrap_or(StateValue::Empty),
            StateKey::ClockValue(id) => self
                .state
                .clocks
                .get(id)
                .map(|clock| StateValue::Number(clock.value))
                .unwrap_or(StateValue::Empty),
            StateKey::ClockPaused(id) => self
                .state
                .clocks
                .get(id)
                .map(|clock| StateValue::Bool(clock.paused))
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
        }
    }

    fn meter_value(&self, actor_id: &str, key: &str) -> StateValue {
        match self.state.meter_value(&self.canon, actor_id, key) {
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
            .map(|actor| StateValue::Number(read(actor.act())))
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

    pub(crate) fn savepoint(&self) -> Savepoint {
        Savepoint {
            state: self.state.clone(),
            keys: self.keys.clone(),
            pending: self.priors.clone(),
            memories: self.memories.clone(),
            memory_writes: self.memory_writes,
        }
    }

    pub(crate) fn rollback(&mut self, point: Savepoint) {
        self.state = point.state;
        self.keys = point.keys;
        self.priors = point.pending;
        self.memories = point.memories;
        self.memory_writes = point.memory_writes;
    }
}

#[derive(Clone)]
struct PriorValue {
    key: StateKey,
    before: StateValue,
}

pub(crate) struct Savepoint {
    state: State,
    keys: BTreeSet<StateKey>,
    pending: Vec<PriorValue>,
    memories: Vec<MemoryLine>,
    memory_writes: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{Intent, Source, Target, Viewer};
    use crate::model::meter::MeterWrite;
    use crate::rules::effect::{Effect, EffectKind};
    use crate::World;

    #[test]
    fn dropping_uncommitted_draft_leaves_save_untouched() {
        let pack =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let save_before = std::fs::read(pack.join("state/save.yaml")).expect("read save");
        let world = World::open(&pack).expect("open");
        let nobody = crate::model::Presence::default();
        let mut draft = Draft::begin(&world, &nobody);
        let intent = Intent {
            id: "i1".to_string(),
            round_index: 1,
            actor: Viewer::Npc("mira".to_string()),
            move_name: "leave_door_note".to_string(),
            target: Target::Empty,
            args: Default::default(),
            rationale_code: "habit".to_string(),
            source: Source::Engine,
        };
        Effect {
            kind: EffectKind::Clock {
                clock_id: "shop_day".to_string(),
                delta: -1,
                start: false,
            },
            viewer: Viewer::Npc("mira".to_string()),
            intent_id: "i1".to_string(),
        }
        .apply(&mut draft, &intent)
        .expect("apply");
        drop(draft);
        drop(world);
        let save_after = std::fs::read(pack.join("state/save.yaml")).expect("read save");
        assert_eq!(save_after, save_before);
    }

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
        let meters = draft.state.actor_mut("bram").unwrap().meters_mut();
        meters.get_mut("grit").unwrap().offset = -5;
        let intent = Intent::habit(world.state(), "mira", "leave_door_note");
        let mut write = |actor: &str, key: &str, delta: i64| {
            let kind = EffectKind::Meter {
                actor_id: actor.to_string(),
                key: key.to_string(),
                write: MeterWrite::Offset(delta),
            };
            Effect::new(&intent.actor, &intent.id, kind).apply(&mut draft, &intent)
        };
        assert_eq!(write("mira", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("mira", "rest", 1), Ok(()), "the first key");
        assert_eq!(write("bram", "grit", -1), Ok(()), "offset moved: key two");
        assert_eq!(write("mira", "rest", 1), Ok(()), "a counted key is free");
        assert_eq!(write("aldric", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("aldric", "rest", 1), Err(Reason::Budget));
    }
}
