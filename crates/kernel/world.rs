use std::collections::BTreeSet;
use std::fs;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::{bind_inventory, bind_meters, load_canon, pack_rev, Canon};
use crate::codec::{emit_now, load_now};
use crate::diagnostic::{coded, locate_nth, reason_of, schema, Diagnostic};
use crate::error::Reason;
use crate::intent::Intent;
use crate::key::StateKey;
use crate::limits::Budget;
use crate::model::{MemoryLine, Now, PauseView};
use crate::recover::recover;
use crate::rules::RollReport;

pub struct World {
    pub(crate) root: PathBuf,
    canon: Arc<Canon>,
    pub(crate) now: Now,
    pub(crate) queue: Vec<Intent>,
    pub(crate) pending: Option<UncommittedRound>,
    rolls: Vec<RollReport>,
}

/// One round played and not yet written. A tick is only the time unit.
pub struct UncommittedRound {
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

pub struct Draft {
    canon: Arc<Canon>,
    budget: Budget,
    pub(crate) state: Now,
    keys: BTreeSet<StateKey>,
    pub(crate) memories: Vec<MemoryLine>,
    memory_writes: u32,
    _nosend: PhantomData<Rc<()>>,
}

impl World {
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

    pub fn queue(&mut self, intent: Intent) {
        self.queue.push(intent);
    }

    pub fn take_rolls(&mut self) -> Vec<RollReport> {
        std::mem::take(&mut self.rolls)
    }

    pub fn open(root: &Path) -> Result<Self, Reason> {
        load_world(root, false)
    }

    /// Open a temp pack for replay. A base from before migrate may not match `pack_rev`.
    pub fn open_for_replay(root: &Path) -> Result<Self, Reason> {
        load_world(root, true)
    }

    /// Hold one played round when the caller will write it. Spook queues intents and takes rolls; it does not write this.
    pub fn finish_round(
        &mut self,
        draft: Draft,
        flush: bool,
        record: String,
        notices: Vec<RollReport>,
        keep_queue: bool,
    ) -> Result<(), Reason> {
        if flush {
            let tick_index = draft.now().tick_index;
            self.pending = Some(UncommittedRound {
                now: draft.now().clone(),
                tick_jsonl: record,
                tick_index,
                memories: draft.memories().to_vec(),
                paths: draft.changes(),
            });
            self.rolls = notices;
            return Ok(());
        }
        crate::commit::commit_round(self, draft, &record)?;
        self.rolls = notices;
        if !keep_queue {
            self.queue.clear();
        }
        Ok(())
    }
}

impl Draft {
    pub fn begin(world: &World) -> Self {
        Self {
            canon: Arc::clone(&world.canon),
            budget: world.canon.budget,
            state: world.now.clone(),
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

    pub fn set_pause_view(&mut self, pause: PauseView) {
        self.state.pause = pause;
    }

    pub fn reset_idle_acts(&mut self) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            let Some(current) = self
                .state
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
                .state
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
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            let Some(current) = self
                .state
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
                .state
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
        let tick_index = self.state.tick_index;
        let (current_idle, current_live, current_last) = {
            let tick = self
                .state
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
            .state
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

    pub fn changes(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.keys.iter().map(ToString::to_string).collect();
        lines.sort();
        lines
    }

    pub fn savepoint(&self) -> DraftPoint {
        DraftPoint {
            state: self.state.clone(),
            keys: self.keys.clone(),
            memories: self.memories.clone(),
            memory_writes: self.memory_writes,
        }
    }

    pub fn rollback(&mut self, point: DraftPoint) {
        self.state = point.state;
        self.keys = point.keys;
        self.memories = point.memories;
        self.memory_writes = point.memory_writes;
    }
}

pub struct DraftPoint {
    state: Now,
    keys: BTreeSet<StateKey>,
    memories: Vec<MemoryLine>,
    memory_writes: u32,
}

pub fn session_status(now: &Now) -> SessionStatus {
    SessionStatus {
        humans: now.presence.humans,
        actors_present: now.presence.actors_present.clone(),
    }
}

fn load_world(root: &Path, replay: bool) -> Result<World, Reason> {
    if !root.join("pack.yaml").is_file() || !root.join("state/now.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    recover(root)?;
    let canon = Arc::new(load_canon(root).map_err(reason_of)?);
    let now_bytes = fs::read(root.join("state/now.yaml")).map_err(|_| Reason::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
    let mut now = load_now(now_text).map_err(reason_of)?;
    let mut problems = save_problems(root, &now, &now_bytes);
    if replay {
        problems.retain(|diag| diag.code != Reason::PackRevMismatch.token());
    }
    if !problems.is_empty() {
        return Err(reason_of(problems));
    }
    bind_meters(&canon, &mut now).map_err(reason_of)?;
    bind_inventory(&now).map_err(reason_of)?;
    Ok(World {
        root: root.to_path_buf(),
        canon,
        now,
        queue: Vec::new(),
        pending: None,
        rolls: Vec::new(),
    })
}

/// Read the pack and report every loader and lint problem. Writes nothing.
pub fn check_pack(root: &Path) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let canon = match load_canon(root) {
        Ok(canon) => Some(canon),
        Err(more) => {
            diags.extend(more);
            None
        }
    };
    let now_path = root.join("state/now.yaml");
    let loaded = if now_path.is_file() {
        match fs::read(&now_path) {
            Ok(bytes) => match std::str::from_utf8(&bytes) {
                Ok(text) => match load_now(text) {
                    Ok(now) => Some((bytes, now)),
                    Err(more) => {
                        diags.extend(more);
                        None
                    }
                },
                Err(_) => {
                    diags.push(
                        schema(
                            "state/now.yaml",
                            "this file is not UTF-8 text",
                            "save it as UTF-8",
                        )
                        .with_path("schema"),
                    );
                    None
                }
            },
            Err(_) => {
                diags.push(
                    coded(
                        "state/now.yaml",
                        Reason::Schema,
                        "state/now.yaml could not be read",
                        "check that the file is readable",
                    )
                    .with_path("schema"),
                );
                None
            }
        }
    } else {
        None
    };
    if let Some((bytes, now)) = &loaded {
        diags.extend(save_problems(root, now, bytes));
        if let Some(canon) = &canon {
            let mut copy = now.clone();
            if let Err(more) = bind_meters(canon, &mut copy) {
                diags.extend(more);
            }
            if let Err(more) = bind_inventory(&copy) {
                diags.extend(more);
            }
        }
    }
    if let Some(canon) = &canon {
        diags.extend(crate::lint::body_leaks(root, canon));
        diags.extend(crate::lint::message_text_leaks(root, canon));
    }
    diags
}

fn save_problems(root: &Path, now: &Now, bytes: &[u8]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    match pack_rev(root) {
        Ok(rev) if now.pack_rev != rev => {
            let mut diag = coded(
                "state/now.yaml",
                Reason::PackRevMismatch,
                "pack_rev does not match the canon hash",
                "run spook migrate to store the current hash",
            )
            .with_path("pack_rev");
            if let Ok(text) = std::str::from_utf8(bytes) {
                if let Some((line, column)) = locate_nth(text, "pack_rev:", 0) {
                    diag = diag.at_line(line, Some(column));
                }
            }
            diags.push(diag);
        }
        Ok(_) => {}
        Err(_) => diags.push(
            coded(
                "state/now.yaml",
                Reason::Schema,
                "the canon files could not be hashed",
                "check that world/, cast/, and rules/ are readable",
            )
            .with_path("pack_rev"),
        ),
    }
    match emit_now(now) {
        Ok(emitted) if emitted.as_bytes() != bytes => {
            let line = std::str::from_utf8(bytes)
                .ok()
                .and_then(|live| first_diff_line(live, &emitted));
            let mut diag = coded(
                "state/now.yaml",
                Reason::HandEdit,
                "this save does not match the canonical text",
                "run spook migrate to keep the edit, or restore the file",
            );
            diag = match line {
                Some(line) => diag.at_line(line, None),
                None => diag.with_path("now"),
            };
            diags.push(diag);
        }
        Ok(_) => {}
        Err(_) => diags.push(
            coded(
                "state/now.yaml",
                Reason::Schema,
                "the save could not be written back as YAML",
                "fix the values so they match the now format",
            )
            .with_path("now"),
        ),
    }
    diags
}

fn first_diff_line(live: &str, emitted: &str) -> Option<u32> {
    for (index, (left, right)) in live.lines().zip(emitted.lines()).enumerate() {
        if left != right {
            return Some(index as u32 + 1);
        }
    }
    if live.lines().count() != emitted.lines().count() {
        return Some(live.lines().count().min(emitted.lines().count()) as u32 + 1);
    }
    None
}
