//! The staged save a round writes before commit.

use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::world_settings::Budget;
use crate::canon::Canon;
use crate::error::Reason;
use crate::key::{ChangeRecord, StateKey, StateValue};
use crate::model::{MemoryLine, Now, PauseView};

pub struct Draft {
    canon: Arc<Canon>,
    budget: Budget,
    pub(crate) state: Now,
    keys: BTreeSet<StateKey>,
    pending: Vec<Noted>,
    pub(crate) memories: Vec<MemoryLine>,
    memory_writes: u32,
    _nosend: PhantomData<Rc<()>>,
}

impl Draft {
    pub(crate) fn from_parts(canon: Arc<Canon>, budget: Budget, state: Now) -> Self {
        Self {
            canon,
            budget,
            state,
            keys: BTreeSet::new(),
            pending: Vec::new(),
            memories: Vec::new(),
            memory_writes: 0,
            _nosend: PhantomData,
        }
    }

    pub fn canon(&self) -> &Canon {
        &self.canon
    }

    pub fn now(&self) -> &Now {
        &self.state
    }

    pub fn budget(&self) -> &Budget {
        &self.budget
    }

    pub(crate) fn now_mut(&mut self) -> &mut Now {
        &mut self.state
    }

    pub fn pin_loop(&mut self, tick_index: u64, world_time: u64, last_tick_at: &str) {
        self.state.tick_index = tick_index;
        self.state.world_time = world_time;
        self.state.last_tick_at = last_tick_at.to_string();
    }

    pub(crate) fn set_pause_view(&mut self, pause: PauseView) {
        self.state.pause = pause;
    }

    pub fn reset_idle_acts(&mut self) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            let current = self
                .state
                .actors
                .get(&id)
                .map(|actor| actor.act().idle_acts_today)
                .unwrap_or(0);
            if current != 0 {
                self.track(StateKey::TickIdleActs(id.clone()))?;
            }
            if let Some(actor) = self.state.actors.get_mut(&id) {
                actor.act_mut().idle_acts_today = 0;
            }
        }
        Ok(())
    }

    pub fn reset_live_acts(&mut self) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            let current = self
                .state
                .actors
                .get(&id)
                .map(|actor| actor.act().live_acts_this_hour)
                .unwrap_or(0);
            if current != 0 {
                self.track(StateKey::TickLiveActs(id.clone()))?;
            }
            if let Some(actor) = self.state.actors.get_mut(&id) {
                actor.act_mut().live_acts_this_hour = 0;
            }
        }
        Ok(())
    }

    pub fn stage_npc_counters(&mut self, actor_id: &str, humans: u32) -> Result<(), Reason> {
        let tick_index = self.state.tick_index;
        let (current_idle, current_live, current_last) = {
            let tick = self
                .state
                .actors
                .get(actor_id)
                .ok_or(Reason::UnknownKey)?
                .act();
            (
                tick.idle_acts_today,
                tick.live_acts_this_hour,
                tick.last_act_tick,
            )
        };
        let new_idle = current_idle.saturating_add(1);
        let new_live = current_live.saturating_add(1);
        let mut keys = Vec::new();
        if tick_index != current_last {
            keys.push(StateKey::TickLastAct(actor_id.to_string()));
        }
        if humans == 0 && new_idle != current_idle {
            keys.push(StateKey::TickIdleActs(actor_id.to_string()));
        }
        if humans > 0 && new_live != current_live {
            keys.push(StateKey::TickLiveActs(actor_id.to_string()));
        }
        self.track_many(&keys)?;
        let tick = self
            .state
            .actors
            .get_mut(actor_id)
            .ok_or(Reason::UnknownKey)?
            .act_mut();
        tick.last_act_tick = tick_index;
        if humans == 0 {
            tick.idle_acts_today = new_idle;
        } else {
            tick.live_acts_this_hour = new_live;
        }
        Ok(())
    }

    pub(crate) fn track(&mut self, key: StateKey) -> Result<(), Reason> {
        self.track_many(std::slice::from_ref(&key))
    }

    /// Record `before` as this key's value from before the write that just landed.
    pub(crate) fn track_before(&mut self, key: StateKey, before: StateValue) -> Result<(), Reason> {
        self.note(key, Some(before))
    }

    pub(crate) fn track_many(&mut self, keys: &[StateKey]) -> Result<(), Reason> {
        let max = self.budget.max_state_keys_mutated as usize;
        let mut added = Vec::new();
        for key in keys {
            self.admit(key, &mut added, max)?;
            if !self.pending.iter().any(|noted| &noted.key == key) {
                let before = self.read_value(key);
                self.pending.push(Noted {
                    key: key.clone(),
                    before,
                });
            }
        }
        Ok(())
    }

    fn note(&mut self, key: StateKey, before: Option<StateValue>) -> Result<(), Reason> {
        let max = self.budget.max_state_keys_mutated as usize;
        let mut added = Vec::new();
        self.admit(&key, &mut added, max)?;
        if !self.pending.iter().any(|noted| noted.key == key) {
            let before = before.unwrap_or_else(|| self.read_value(&key));
            self.pending.push(Noted { key, before });
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
                self.pending.retain(|noted| &noted.key != key);
            }
            return Err(Reason::Budget);
        }
        self.keys.insert(key.clone());
        added.push(key.clone());
        Ok(())
    }

    /// Changes since the last take. The cumulative key set used by `changes` stays.
    pub fn take_changes(&mut self) -> Vec<ChangeRecord> {
        let pending = std::mem::take(&mut self.pending);
        let mut records = Vec::new();
        for noted in pending {
            let after = self.read_value(&noted.key);
            if after == noted.before {
                continue;
            }
            records.push(ChangeRecord {
                key: noted.key,
                before: noted.before,
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
            StateKey::TickLastAct(id) => self.act_number(id, |act| {
                i64::try_from(act.last_act_tick).unwrap_or(i64::MAX)
            }),
            StateKey::TickIdleActs(id) => self.act_number(id, |act| i64::from(act.idle_acts_today)),
            StateKey::TickLiveActs(id) => {
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
        let Some(def) = self.canon.meters.get(key) else {
            return StateValue::Empty;
        };
        let Some(live) = self
            .state
            .actors
            .get(actor_id)
            .and_then(|actor| actor.meters().get(key))
        else {
            return StateValue::Empty;
        };
        match crate::meter::current(def, live) {
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

    pub fn memories(&self) -> &[MemoryLine] {
        &self.memories
    }

    pub fn changes(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.keys.iter().map(ToString::to_string).collect();
        lines.sort();
        lines
    }

    pub fn savepoint(&self) -> DraftPoint {
        DraftPoint {
            state: self.state.clone(),
            keys: self.keys.clone(),
            pending: self.pending.clone(),
            memories: self.memories.clone(),
            memory_writes: self.memory_writes,
        }
    }

    pub fn rollback(&mut self, point: DraftPoint) {
        self.state = point.state;
        self.keys = point.keys;
        self.pending = point.pending;
        self.memories = point.memories;
        self.memory_writes = point.memory_writes;
    }
}

#[derive(Clone)]
struct Noted {
    key: StateKey,
    before: StateValue,
}

pub struct DraftPoint {
    state: Now,
    keys: BTreeSet<StateKey>,
    pending: Vec<Noted>,
    memories: Vec<MemoryLine>,
    memory_writes: u32,
}
