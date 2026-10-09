use std::collections::BTreeSet;
use std::fs;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::{bind_inventory, bind_meters, load_canon, pack_rev, Canon};
use crate::check::CheckNotice;
use crate::codec::{emit_now, load_now};
use crate::error::Reason;
use crate::intent::Intent;
use crate::key::StateKey;
use crate::limits::Budget;
use crate::model::{MemoryLine, Now, PauseView};
use crate::recover::recover;

pub struct Shard {
    pub(crate) root: PathBuf,
    canon: Arc<Canon>,
    pub(crate) now: Now,
    pub(crate) queue: Vec<Intent>,
    pub(crate) pending: Option<PendingCommit>,
    rolls: Vec<CheckNotice>,
}

pub struct PendingCommit {
    pub now: Now,
    pub tick_jsonl: String,
    pub tick_index: u64,
    pub memories: Vec<MemoryLine>,
    pub paths: Vec<String>,
}

pub struct SessionStatus {
    pub humans: u32,
    pub actors_present: Vec<String>,
}

pub struct Txn {
    canon: Arc<Canon>,
    budget: Budget,
    pub(crate) staging: Now,
    keys: BTreeSet<StateKey>,
    pub(crate) memories: Vec<MemoryLine>,
    memory_writes: u32,
    _nosend: PhantomData<Rc<()>>,
}

impl Shard {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn canon(&self) -> &Canon {
        &self.canon
    }

    pub fn now(&self) -> &Now {
        &self.now
    }

    pub fn budget(&self) -> &Budget {
        &self.canon.budget
    }

    pub fn queued_intents(&self) -> &[Intent] {
        &self.queue
    }

    pub fn queue_intent(&mut self, intent: Intent) {
        self.queue.push(intent);
    }

    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }

    pub fn set_pending(&mut self, pending: PendingCommit) {
        self.pending = Some(pending);
    }

    pub fn take_pending(&mut self) -> Option<PendingCommit> {
        self.pending.take()
    }

    pub fn set_rolls(&mut self, rolls: Vec<CheckNotice>) {
        self.rolls = rolls;
    }

    pub fn take_rolls(&mut self) -> Vec<CheckNotice> {
        std::mem::take(&mut self.rolls)
    }
}

impl Txn {
    pub fn begin(shard: &Shard) -> Self {
        Self {
            canon: Arc::clone(&shard.canon),
            budget: shard.canon.budget,
            staging: shard.now.clone(),
            keys: BTreeSet::new(),
            memories: Vec::new(),
            memory_writes: 0,
            _nosend: PhantomData,
        }
    }

    pub fn canon(&self) -> &Canon {
        &self.canon
    }

    pub fn now(&self) -> &Now {
        &self.staging
    }

    pub fn budget(&self) -> &Budget {
        &self.budget
    }

    pub(crate) fn now_mut(&mut self) -> &mut Now {
        &mut self.staging
    }

    pub fn pin_loop(&mut self, tick_index: u64, world_time: u64, last_tick_at: &str) {
        self.staging.tick_index = tick_index;
        self.staging.world_time = world_time;
        self.staging.last_tick_at = last_tick_at.to_string();
    }

    pub fn set_pause_view(&mut self, pause: PauseView) {
        self.staging.pause = pause;
    }

    pub fn reset_idle_acts(&mut self) -> Result<(), Reason> {
        let ids: Vec<String> = self.staging.actors.keys().cloned().collect();
        for id in ids {
            let Some(current) = self
                .staging
                .actors
                .get(&id)
                .and_then(|actor| actor.tick())
                .map(|tick| tick.idle_acts_today)
            else {
                continue;
            };
            if current != 0 {
                self.track(StateKey::TickIdleActs(id.clone()))?;
            }
            if let Some(tick) = self
                .staging
                .actors
                .get_mut(&id)
                .and_then(|actor| actor.tick_mut())
            {
                tick.idle_acts_today = 0;
            }
        }
        Ok(())
    }

    pub fn reset_live_acts(&mut self) -> Result<(), Reason> {
        let ids: Vec<String> = self.staging.actors.keys().cloned().collect();
        for id in ids {
            let Some(current) = self
                .staging
                .actors
                .get(&id)
                .and_then(|actor| actor.tick())
                .map(|tick| tick.live_acts_this_hour)
            else {
                continue;
            };
            if current != 0 {
                self.track(StateKey::TickLiveActs(id.clone()))?;
            }
            if let Some(tick) = self
                .staging
                .actors
                .get_mut(&id)
                .and_then(|actor| actor.tick_mut())
            {
                tick.live_acts_this_hour = 0;
            }
        }
        Ok(())
    }

    pub fn stage_npc_counters(&mut self, actor_id: &str, humans: u32) -> Result<(), Reason> {
        let tick_index = self.staging.tick_index;
        let (current_idle, current_live, current_last) = {
            let tick = self
                .staging
                .actors
                .get(actor_id)
                .and_then(|actor| actor.tick())
                .ok_or(Reason::NoProfile)?;
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
            .staging
            .actors
            .get_mut(actor_id)
            .and_then(|actor| actor.tick_mut())
            .ok_or(Reason::NoProfile)?;
        tick.last_act_tick = tick_index;
        if humans == 0 {
            tick.idle_acts_today = new_idle;
        } else {
            tick.live_acts_this_hour = new_live;
        }
        Ok(())
    }

    pub fn track(&mut self, key: StateKey) -> Result<(), Reason> {
        self.track_many(std::slice::from_ref(&key))
    }

    pub fn track_many(&mut self, keys: &[StateKey]) -> Result<(), Reason> {
        let max = self.budget.max_state_keys_mutated as usize;
        let mut added = Vec::new();
        for key in keys {
            if self.keys.contains(key) {
                continue;
            }
            if self.keys.len() >= max {
                for key in &added {
                    self.keys.remove(key);
                }
                return Err(Reason::Budget);
            }
            self.keys.insert(key.clone());
            added.push(key.clone());
        }
        Ok(())
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

    pub fn staged_paths(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.keys.iter().map(ToString::to_string).collect();
        lines.sort();
        lines
    }
}

pub fn session_status(now: &Now) -> SessionStatus {
    SessionStatus {
        humans: now.presence.humans,
        actors_present: now.presence.actors_present.clone(),
    }
}

pub fn open_shard(root: &Path) -> Result<Shard, Reason> {
    if !root.join("pack.yaml").is_file() || !root.join("state/now.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    recover(root)?;
    let canon = Arc::new(load_canon(root)?);
    let now_bytes = fs::read(root.join("state/now.yaml")).map_err(|_| Reason::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
    let mut now = load_now(now_text)?;
    let rev = pack_rev(root)?;
    if now.pack_rev != rev {
        return Err(Reason::PackRevMismatch);
    }
    let emitted = emit_now(&now)?;
    if emitted.as_bytes() != now_bytes {
        return Err(Reason::HandEdit);
    }
    bind_meters(&canon, &mut now)?;
    bind_inventory(&now)?;
    Ok(Shard {
        root: root.to_path_buf(),
        canon,
        now,
        queue: Vec::new(),
        pending: None,
        rolls: Vec::new(),
    })
}
