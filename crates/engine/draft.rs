//! The staged save a round writes before commit.

use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::Canon;
use crate::canon::{Budget, PausePolicy};
use crate::error::Reason;
use crate::key::{ChangeRecord, StateKey, StateValue};
use crate::model::{MemoryLine, Now, Presence, RunState};

pub(crate) struct Draft<'p> {
    canon: Arc<Canon>,
    budget: Budget,
    pub(crate) state: Now,
    /// Who is seated for this round. The round reads it and never writes it.
    pub(crate) presence: &'p Presence,
    keys: BTreeSet<StateKey>,
    pending: Vec<PriorValue>,
    pub(crate) memories: Vec<MemoryLine>,
    memory_writes: u32,
    _nosend: PhantomData<Rc<()>>,
}

impl<'p> Draft<'p> {
    pub(crate) fn from_parts(
        canon: Arc<Canon>,
        budget: Budget,
        state: Now,
        presence: &'p Presence,
    ) -> Self {
        Self {
            canon,
            budget,
            state,
            presence,
            keys: BTreeSet::new(),
            pending: Vec::new(),
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

    pub(crate) fn now(&self) -> &Now {
        &self.state
    }

    pub(crate) fn budget(&self) -> &Budget {
        &self.budget
    }

    pub(crate) fn now_mut(&mut self) -> &mut Now {
        &mut self.state
    }

    pub(crate) fn stamp_round(&mut self, round_index: u64, world_time: u64, last_round_at: &str) {
        self.state.round_index = round_index;
        self.state.world_time = world_time;
        self.state.last_round_at = last_round_at.to_string();
    }

    pub(crate) fn apply_pause_policy(&mut self) -> Result<(), Reason> {
        let humans = self.presence.humans;
        let policies = self.budget.pause;
        let clock_ids: Vec<String> = self.state.clocks.keys().cloned().collect();
        for clock_id in clock_ids {
            let Some(clock) = self.state.clocks.get(&clock_id) else {
                continue;
            };
            if !clock.requires_presence {
                continue;
            }
            let paused = humans == 0 && policies.named_plot_clocks == PausePolicy::PauseWhenEmpty;
            if humans > 0 {
                self.set_clock_paused(&clock_id, false)?;
            } else if paused {
                self.set_clock_paused(&clock_id, true)?;
            }
        }
        self.state.pause.named_plot_clocks = match policies.named_plot_clocks {
            PausePolicy::PauseWhenEmpty if humans == 0 => RunState::Paused,
            PausePolicy::PauseWhenEmpty | PausePolicy::AlwaysRun => RunState::Running,
        };
        Ok(())
    }

    fn set_clock_paused(&mut self, clock_id: &str, paused: bool) -> Result<(), Reason> {
        let current = self.state.clocks.get(clock_id).map(|clock| clock.paused);
        if current != Some(paused) {
            self.track(StateKey::ClockPaused(clock_id.to_string()))?;
        }
        if let Some(clock) = self.state.clocks.get_mut(clock_id) {
            clock.paused = paused;
        }
        Ok(())
    }

    pub(crate) fn reset_idle_acts(&mut self) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            let current = self
                .state
                .actors
                .get(&id)
                .map(|actor| actor.act().idle_acts_today)
                .unwrap_or(0);
            if current != 0 {
                self.track(StateKey::IdleActsToday(id.clone()))?;
            }
            if let Some(actor) = self.state.actors.get_mut(&id) {
                actor.act_mut().idle_acts_today = 0;
            }
        }
        Ok(())
    }

    pub(crate) fn reset_live_acts(&mut self) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            let current = self
                .state
                .actors
                .get(&id)
                .map(|actor| actor.act().live_acts_this_hour)
                .unwrap_or(0);
            if current != 0 {
                self.track(StateKey::LiveActsThisHour(id.clone()))?;
            }
            if let Some(actor) = self.state.actors.get_mut(&id) {
                actor.act_mut().live_acts_this_hour = 0;
            }
        }
        Ok(())
    }

    pub(crate) fn stage_npc_counters(&mut self, actor_id: &str, humans: u32) -> Result<(), Reason> {
        let round_index = self.state.round_index;
        let (current_idle, current_live, current_last) = {
            let counters = self
                .state
                .actors
                .get(actor_id)
                .ok_or(Reason::UnknownKey)?
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
        if humans == 0 && new_idle != current_idle {
            keys.push(StateKey::IdleActsToday(actor_id.to_string()));
        }
        if humans > 0 && new_live != current_live {
            keys.push(StateKey::LiveActsThisHour(actor_id.to_string()));
        }
        self.track_many(&keys)?;
        let counters = self
            .state
            .actors
            .get_mut(actor_id)
            .ok_or(Reason::UnknownKey)?
            .act_mut();
        counters.last_act_round = round_index;
        if humans == 0 {
            counters.idle_acts_today = new_idle;
        } else {
            counters.live_acts_this_hour = new_live;
        }
        Ok(())
    }

    pub(crate) fn track(&mut self, key: StateKey) -> Result<(), Reason> {
        self.track_many(std::slice::from_ref(&key))
    }

    /// Record `before` as this key's value from before the write that just landed.
    pub(crate) fn track_before(&mut self, key: StateKey, before: StateValue) -> Result<(), Reason> {
        self.record_prior(key, Some(before))
    }

    pub(crate) fn track_many(&mut self, keys: &[StateKey]) -> Result<(), Reason> {
        let max = self.budget.max_state_keys_mutated as usize;
        let mut added = Vec::new();
        for key in keys {
            self.admit(key, &mut added, max)?;
            if !self.pending.iter().any(|prior| &prior.key == key) {
                let before = self.read_value(key);
                self.pending.push(PriorValue {
                    key: key.clone(),
                    before,
                });
            }
        }
        Ok(())
    }

    fn record_prior(&mut self, key: StateKey, before: Option<StateValue>) -> Result<(), Reason> {
        let max = self.budget.max_state_keys_mutated as usize;
        let mut added = Vec::new();
        self.admit(&key, &mut added, max)?;
        if !self.pending.iter().any(|prior| prior.key == key) {
            let before = before.unwrap_or_else(|| self.read_value(&key));
            self.pending.push(PriorValue { key, before });
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
                self.pending.retain(|prior| &prior.key != key);
            }
            return Err(Reason::Budget);
        }
        self.keys.insert(key.clone());
        added.push(key.clone());
        Ok(())
    }

    /// Changes since the last take. The cumulative key set used by `changes` stays.
    pub(crate) fn take_changes(&mut self) -> Vec<ChangeRecord> {
        let pending = std::mem::take(&mut self.pending);
        let mut records = Vec::new();
        for prior in pending {
            let after = self.read_value(&prior.key);
            if after == prior.before {
                continue;
            }
            records.push(ChangeRecord {
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
                .map(|actor| StateValue::Mark(actor.flags().iter().any(|item| item == flag)))
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
                .map(|actor| StateValue::Number(i64::from(actor.count(item))))
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
                .map(|clock| StateValue::Mark(clock.paused))
                .unwrap_or(StateValue::Empty),
            StateKey::Message(id) => {
                if self.state.messages.iter().any(|message| &message.id == id) {
                    StateValue::Mark(true)
                } else {
                    StateValue::Empty
                }
            }
            StateKey::WorldFlag(key) => {
                StateValue::Mark(self.state.flags.iter().any(|flag| flag == key))
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
        read: impl FnOnce(&crate::model::ActCounters) -> i64,
    ) -> StateValue {
        self.state
            .actors
            .get(actor_id)
            .map(|actor| StateValue::Number(read(actor.act())))
            .unwrap_or(StateValue::Empty)
    }

    pub(crate) fn bump_memory(&mut self) -> Result<(), Reason> {
        if u64::from(self.memory_writes) >= self.budget.max_memory_writes {
            return Err(Reason::Budget);
        }
        self.memory_writes += 1;
        Ok(())
    }

    pub(crate) fn stage_memory(&mut self, line: MemoryLine) {
        self.memories.push(line);
    }

    pub(crate) fn savepoint(&self) -> DraftPoint {
        DraftPoint {
            state: self.state.clone(),
            keys: self.keys.clone(),
            pending: self.pending.clone(),
            memories: self.memories.clone(),
            memory_writes: self.memory_writes,
        }
    }

    pub(crate) fn rollback(&mut self, point: DraftPoint) {
        self.state = point.state;
        self.keys = point.keys;
        self.pending = point.pending;
        self.memories = point.memories;
        self.memory_writes = point.memory_writes;
    }
}

#[derive(Clone)]
struct PriorValue {
    key: StateKey,
    before: StateValue,
}

pub(crate) struct DraftPoint {
    state: Now,
    keys: BTreeSet<StateKey>,
    pending: Vec<PriorValue>,
    memories: Vec<MemoryLine>,
    memory_writes: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::apply_effect;
    use crate::effect::{Effect, EffectKind};
    use crate::intent::{Intent, Source, Target, ViewerId};
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
            actor: ViewerId::Npc("mira".to_string()),
            move_name: "leave_door_note".to_string(),
            target: Target::Empty,
            args: Default::default(),
            rationale_code: "habit".to_string(),
            source: Source::Kernel,
        };
        apply_effect(
            &mut draft,
            &intent,
            Effect {
                class: EffectKind::Clock {
                    clock_id: "shop_day".to_string(),
                    delta: -1,
                    start: false,
                },
                as_viewer: ViewerId::Npc("mira".to_string()),
                intent_id: "i1".to_string(),
            },
        )
        .expect("apply_effect");
        drop(draft);
        drop(world);
        let save_after = std::fs::read(pack.join("state/save.yaml")).expect("read save");
        assert_eq!(save_after, save_before);
    }
}
