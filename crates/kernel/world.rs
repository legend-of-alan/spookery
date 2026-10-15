use std::collections::BTreeSet;
use std::fs;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::world_settings::Budget;
use crate::canon::{bind_inventory, bind_meters, load_canon, pack_rev, Canon};
use crate::diagnostic::{coded, locate_nth, reason_of, schema, Diagnostic};
use crate::error::Reason;
use crate::intent::Intent;
use crate::key::{ChangeRecord, StateKey, StateValue};
use crate::model::{MemoryLine, Now, PauseView};
use crate::rules::RollReport;
use crate::store::codec::{emit_save, load_save};
use crate::store::recover::recover;
use crate::visibility::{Seen, Told};

pub struct World {
    pub(crate) root: PathBuf,
    canon: Arc<Canon>,
    pub(crate) now: Now,
    pub(crate) queue: Vec<Intent>,
    pub(crate) pending: Option<UncommittedRound>,
    rolls: Vec<RollReport>,
    summary: Vec<String>,
    lines: Vec<Told>,
    seen: Vec<Seen>,
}

/// What one round showed its viewers.
pub struct Delivered {
    pub summary: Vec<String>,
    pub lines: Vec<Told>,
    pub seen: Vec<Seen>,
}

/// One round played and not yet written. A tick is only the time unit.
pub struct UncommittedRound {
    pub now: Now,
    pub round_jsonl: String,
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
    pending: Vec<Noted>,
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

    /// Change keys from the latest round. The GM sees every one.
    pub fn change_summary(&self) -> &[String] {
        &self.summary
    }

    /// Change keys each actor can see, from rounds since the last take.
    pub fn seen(&self) -> &[Seen] {
        &self.seen
    }

    pub fn take_lines(&mut self) -> Vec<Told> {
        std::mem::take(&mut self.lines)
    }

    pub fn take_seen(&mut self) -> Vec<Seen> {
        std::mem::take(&mut self.seen)
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
        delivered: Delivered,
    ) -> Result<(), Reason> {
        self.summary = delivered.summary.clone();
        self.lines.extend(delivered.lines);
        self.seen.extend(delivered.seen);
        if flush {
            let tick_index = draft.now().tick_index;
            self.pending = Some(UncommittedRound {
                now: draft.now().clone(),
                round_jsonl: record,
                tick_index,
                memories: draft.memories().to_vec(),
                paths: delivered.summary,
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

    pub fn set_pause_view(&mut self, pause: PauseView) {
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

    pub fn track(&mut self, key: StateKey) -> Result<(), Reason> {
        self.track_many(std::slice::from_ref(&key))
    }

    /// Record `before` as this key's value from before the write that just landed.
    pub fn track_before(&mut self, key: StateKey, before: StateValue) -> Result<(), Reason> {
        self.note(key, Some(before))
    }

    pub fn track_many(&mut self, keys: &[StateKey]) -> Result<(), Reason> {
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

pub fn session_status(now: &Now) -> SessionStatus {
    SessionStatus {
        humans: now.presence.humans,
        actors_present: now.presence.actors_present.clone(),
    }
}

fn load_world(root: &Path, replay: bool) -> Result<World, Reason> {
    if !root.join("pack.yaml").is_file() || !root.join("state/save.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    recover(root)?;
    let canon = Arc::new(load_canon(root).map_err(reason_of)?);
    let now_bytes = fs::read(root.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
    let mut now = load_save(now_text).map_err(reason_of)?;
    let mut problems = save_problems(root, &now, &now_bytes);
    if replay {
        problems.retain(|diag| diag.code != Reason::PackRevMismatch.token());
    }
    if !problems.is_empty() {
        return Err(reason_of(problems));
    }
    bind_meters(&canon, &mut now).map_err(reason_of)?;
    bind_inventory(&now).map_err(reason_of)?;
    let unknown = crate::canon::unknown_save_items(&canon, &now);
    if !unknown.is_empty() {
        return Err(reason_of(unknown));
    }
    Ok(World {
        root: root.to_path_buf(),
        canon,
        now,
        queue: Vec::new(),
        pending: None,
        rolls: Vec::new(),
        summary: Vec::new(),
        lines: Vec::new(),
        seen: Vec::new(),
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
    let now_path = root.join("state/save.yaml");
    let loaded = if now_path.is_file() {
        match fs::read(&now_path) {
            Ok(bytes) => match std::str::from_utf8(&bytes) {
                Ok(text) => match load_save(text) {
                    Ok(now) => Some((bytes, now)),
                    Err(more) => {
                        diags.extend(more);
                        None
                    }
                },
                Err(_) => {
                    diags.push(
                        schema(
                            "state/save.yaml",
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
                        "state/save.yaml",
                        Reason::Schema,
                        "state/save.yaml could not be read",
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
            diags.extend(crate::canon::unknown_save_items(canon, &copy));
        }
    }
    if let Some(canon) = &canon {
        diags.extend(crate::lint::body_leaks(root, canon));
        diags.extend(crate::lint::message_text_leaks(root, canon));
        diags.extend(crate::map::unreachable(canon));
        diags.extend(crate::map::teleport_in_verbs(canon));
    }
    diags
}

fn save_problems(root: &Path, now: &Now, bytes: &[u8]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    match pack_rev(root) {
        Ok(rev) if now.pack_rev != rev => {
            let mut diag = coded(
                "state/save.yaml",
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
                "state/save.yaml",
                Reason::Schema,
                "the canon files could not be hashed",
                "check that world/, cast/, and rules/ are readable",
            )
            .with_path("pack_rev"),
        ),
    }
    match emit_save(now) {
        Ok(emitted) if emitted.as_bytes() != bytes => {
            let line = std::str::from_utf8(bytes)
                .ok()
                .and_then(|live| first_diff_line(live, &emitted));
            let mut diag = coded(
                "state/save.yaml",
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
                "state/save.yaml",
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
