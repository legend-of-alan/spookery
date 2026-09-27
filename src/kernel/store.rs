use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::marker::PhantomData;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::{load_canon, pack_rev, Canon};
use crate::codec::{canonicalize, emit_now, emit_proposal, emit_rumors, load_now};
use crate::error::Code;
use crate::limits::Budget;
use crate::model::{Intent, MemoryLine, Now, PauseView, PcSheet, Presence, ProposalDoc};

#[cfg(unix)]
const LOCK_EX: i32 = 2;
#[cfg(unix)]
const LOCK_UN: i32 = 8;
#[cfg(unix)]
const LOCK_NB: i32 = 4;

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: i32, op: i32) -> i32;
}

#[cfg(windows)]
const LOCKFILE_FAIL_IMMEDIATELY: u32 = 0x0000_0001;
#[cfg(windows)]
const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x0000_0002;

#[cfg(windows)]
#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: *mut core::ffi::c_void,
}

#[cfg(windows)]
unsafe extern "system" {
    fn LockFileEx(
        file: *mut core::ffi::c_void,
        flags: u32,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn UnlockFileEx(
        file: *mut core::ffi::c_void,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
}

pub struct Shard {
    root: PathBuf,
    canon: Arc<Canon>,
    now: Now,
    sheets: BTreeMap<String, PcSheet>,
    queue: Vec<Intent>,
    pending: Option<PendingCommit>,
}

pub struct PendingCommit {
    pub now: Now,
    pub tick_jsonl: String,
    pub tick_index: u64,
    pub proposals: Vec<ProposalDoc>,
    pub memories: Vec<MemoryLine>,
    pub paths: Vec<String>,
}

pub struct CommitOutcome {
    pub state_rev: u64,
    pub diff_summary: Vec<String>,
    pub log_path: String,
}

pub struct SessionStatus {
    pub humans: u32,
    pub session_ids: Vec<String>,
    pub actors_present: Vec<String>,
}

pub struct LockGuard {
    file: File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        unlock(&self.file);
    }
}

pub struct Txn {
    canon: Arc<Canon>,
    budget: Budget,
    staging: Now,
    baseline: Now,
    paths: BTreeSet<String>,
    memories: Vec<MemoryLine>,
    proposals: Vec<ProposalDoc>,
    rumor_hops: u32,
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

    pub fn sheets(&self) -> &BTreeMap<String, PcSheet> {
        &self.sheets
    }

    pub fn queued_intents(&self) -> &[Intent] {
        &self.queue
    }

    pub fn queue_intent(&mut self, intent: Intent) {
        self.queue.push(intent);
    }

    pub fn set_pending(&mut self, pending: PendingCommit) {
        self.pending = Some(pending);
    }

    pub fn take_pending(&mut self) -> Option<PendingCommit> {
        self.pending.take()
    }
}

impl Txn {
    pub fn begin(shard: &Shard) -> Self {
        Self {
            canon: Arc::clone(&shard.canon),
            budget: shard.canon.budget,
            staging: shard.now.clone(),
            baseline: shard.now.clone(),
            paths: BTreeSet::new(),
            memories: Vec::new(),
            proposals: Vec::new(),
            rumor_hops: 0,
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

    pub(crate) fn baseline(&self) -> &Now {
        &self.baseline
    }

    pub fn pin_loop(&mut self, tick_index: u64, world_time: u64, last_tick_at: &str) {
        self.staging.tick_index = tick_index;
        self.staging.world_time = world_time;
        self.staging.last_tick_at = last_tick_at.to_string();
    }

    pub fn set_pause_view(&mut self, pause: PauseView) {
        self.staging.pause = pause;
    }

    pub fn reset_idle_acts(&mut self) -> Result<(), Code> {
        let ids: Vec<String> = self.staging.actors.keys().cloned().collect();
        for id in ids {
            let Some(base) = self
                .baseline
                .actors
                .get(&id)
                .and_then(|actor| actor.tick())
                .map(|tick| tick.idle_acts_today)
            else {
                continue;
            };
            if base != 0 {
                self.track(format!("actors.{id}.tick.idle_acts_today"))?;
            }
            if let Some(tick) = self.staging.actors.get_mut(&id).and_then(|actor| actor.tick_mut())
            {
                tick.idle_acts_today = 0;
            }
        }
        Ok(())
    }

    pub fn reset_live_acts(&mut self) -> Result<(), Code> {
        let ids: Vec<String> = self.staging.actors.keys().cloned().collect();
        for id in ids {
            let Some(base) = self
                .baseline
                .actors
                .get(&id)
                .and_then(|actor| actor.tick())
                .map(|tick| tick.live_acts_this_hour)
            else {
                continue;
            };
            if base != 0 {
                self.track(format!("actors.{id}.tick.live_acts_this_hour"))?;
            }
            if let Some(tick) = self.staging.actors.get_mut(&id).and_then(|actor| actor.tick_mut())
            {
                tick.live_acts_this_hour = 0;
            }
        }
        Ok(())
    }

    pub fn stage_npc_counters(&mut self, actor_id: &str, humans: u32) -> Result<(), Code> {
        let tick_index = self.staging.tick_index;
        let (current_idle, current_live) = {
            let tick = self
                .staging
                .actors
                .get(actor_id)
                .and_then(|actor| actor.tick())
                .ok_or(Code::NotOwner)?;
            (tick.idle_acts_today, tick.live_acts_this_hour)
        };
        let base = self
            .baseline
            .actors
            .get(actor_id)
            .and_then(|actor| actor.tick())
            .ok_or(Code::NotOwner)?;
        let base_last = base.last_act_tick;
        let base_idle = base.idle_acts_today;
        let base_live = base.live_acts_this_hour;
        let new_idle = current_idle.saturating_add(1);
        let new_live = current_live.saturating_add(1);
        let mut paths = Vec::new();
        if tick_index != base_last {
            paths.push(format!("actors.{actor_id}.tick.last_act_tick"));
        }
        if humans == 0 && new_idle != base_idle {
            paths.push(format!("actors.{actor_id}.tick.idle_acts_today"));
        }
        if humans > 0 && new_live != base_live {
            paths.push(format!("actors.{actor_id}.tick.live_acts_this_hour"));
        }
        self.track_many(&paths)?;
        let tick = self
            .staging
            .actors
            .get_mut(actor_id)
            .and_then(|actor| actor.tick_mut())
            .ok_or(Code::NotOwner)?;
        tick.last_act_tick = tick_index;
        if humans == 0 {
            tick.idle_acts_today = new_idle;
        } else {
            tick.live_acts_this_hour = new_live;
        }
        Ok(())
    }

    pub fn track(&mut self, path: String) -> Result<(), Code> {
        self.track_many(std::slice::from_ref(&path))
    }

    pub fn track_many(&mut self, paths: &[String]) -> Result<(), Code> {
        let max = self.budget.max_state_keys_mutated as usize;
        let mut added = Vec::new();
        for path in paths {
            if self.paths.contains(path) {
                continue;
            }
            if self.paths.len() >= max {
                for path in &added {
                    self.paths.remove(path);
                }
                return Err(Code::Budget);
            }
            self.paths.insert(path.clone());
            added.push(path.clone());
        }
        Ok(())
    }

    pub(crate) fn bump_memory(&mut self) -> Result<(), Code> {
        if u64::from(self.memory_writes) >= self.budget.max_memory_writes {
            return Err(Code::Budget);
        }
        self.memory_writes += 1;
        Ok(())
    }

    pub(crate) fn bump_rumor(&mut self) -> Result<(), Code> {
        if u64::from(self.rumor_hops) >= self.budget.max_rumor_hops {
            return Err(Code::Budget);
        }
        self.rumor_hops += 1;
        Ok(())
    }

    pub(crate) fn stage_memory(&mut self, line: MemoryLine) {
        self.memories.push(line);
    }

    pub(crate) fn stage_proposal(&mut self, doc: ProposalDoc) {
        self.proposals.push(doc);
    }

    pub fn memories(&self) -> &[MemoryLine] {
        &self.memories
    }

    pub fn proposals(&self) -> &[ProposalDoc] {
        &self.proposals
    }

    pub fn staged_paths(&self) -> Vec<String> {
        self.paths.iter().cloned().collect()
    }
}

pub fn session_status(now: &Now) -> SessionStatus {
    SessionStatus {
        humans: now.presence.humans,
        session_ids: now.presence.session_ids.clone(),
        actors_present: now.presence.actors_present.clone(),
    }
}

pub fn open_shard(root: &Path) -> Result<Shard, Code> {
    if !root.join("pack.yaml").is_file() || !root.join("state/now.yaml").is_file() {
        return Err(Code::NotInSpec);
    }
    recover(root)?;
    let canon = Arc::new(load_canon(root)?);
    let now_bytes = fs::read(root.join("state/now.yaml")).map_err(|_| Code::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Code::Schema)?;
    let now = load_now(now_text)?;
    let rev = pack_rev(root)?;
    if now.pack_rev != rev {
        return Err(Code::PackRevMismatch);
    }
    let emitted = emit_now(&now)?;
    if emitted.as_bytes() != now_bytes {
        return Err(Code::HandEdit);
    }
    let mut sheets = BTreeMap::new();
    for (id, actor) in &now.actors {
        if let crate::model::ActorLive::Pc { sheet, .. } = actor {
            let path = root.join(sheet);
            let text = fs::read_to_string(&path).map_err(|_| Code::Schema)?;
            let loaded: PcSheet = serde_saphyr::from_str(&text).map_err(|_| Code::Schema)?;
            if loaded.schema != "sheet/v0" {
                return Err(Code::Schema);
            }
            sheets.insert(id.clone(), loaded);
        }
    }
    Ok(Shard {
        root: root.to_path_buf(),
        canon,
        now,
        sheets,
        queue: Vec::new(),
        pending: None,
    })
}

pub fn lock_shard(shard: &Shard) -> Result<LockGuard, Code> {
    lock_pack(&shard.root)
}

fn lock_pack(root: &Path) -> Result<LockGuard, Code> {
    let path = root.join("state/.tick.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|_| Code::CommitConflict)?;
    if !try_lock_exclusive(&file) {
        return Err(Code::CommitConflict);
    }
    Ok(LockGuard { file })
}

#[cfg(unix)]
fn try_lock_exclusive(file: &File) -> bool {
    unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) == 0 }
}

#[cfg(unix)]
fn unlock(file: &File) {
    unsafe {
        flock(file.as_raw_fd(), LOCK_UN);
    }
}

#[cfg(windows)]
fn try_lock_exclusive(file: &File) -> bool {
    let mut overlapped = blank_overlapped();
    unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_FAIL_IMMEDIATELY | LOCKFILE_EXCLUSIVE_LOCK,
            0,
            1,
            0,
            &mut overlapped,
        ) != 0
    }
}

#[cfg(windows)]
fn unlock(file: &File) {
    let mut overlapped = blank_overlapped();
    unsafe {
        UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut overlapped);
    }
}

#[cfg(windows)]
fn blank_overlapped() -> Overlapped {
    Overlapped {
        internal: 0,
        internal_high: 0,
        offset: 0,
        offset_high: 0,
        event: core::ptr::null_mut(),
    }
}

pub fn tick_commit(shard: &mut Shard, mut txn: Txn, tick_jsonl: &str) -> Result<(), Code> {
    txn.staging.state_rev = txn
        .staging
        .state_rev
        .checked_add(1)
        .ok_or(Code::CommitConflict)?;
    canonicalize(&mut txn.staging);
    let now_bytes = emit_now(&txn.staging)?.into_bytes();
    let tick_index = txn.staging.tick_index;
    let state = shard.root.join("state");
    let planned = plan_commit(
        &state,
        &txn.staging,
        now_bytes,
        Some((tick_index, tick_jsonl.as_bytes())),
        &txn.memories,
        &txn.proposals,
    )?;
    write_commit(&state, &planned)?;
    shard.now = txn.staging;
    Ok(())
}

/// `include_tick: false` for a presence change. No tick file and no clock advance.
/// Identical presence writes nothing and does not bump `state_rev`.
pub fn commit_presence(shard: &mut Shard, presence: Presence) -> Result<bool, Code> {
    let mut next = shard.now.clone();
    next.presence = presence;
    canonicalize(&mut next);
    if next.presence == shard.now.presence {
        return Ok(false);
    }
    let _lock = lock_shard(shard)?;
    next.state_rev = next.state_rev.checked_add(1).ok_or(Code::CommitConflict)?;
    let now_bytes = emit_now(&next)?.into_bytes();
    let state = shard.root.join("state");
    let planned = vec![Planned {
        target: state.join("now.yaml"),
        bytes: now_bytes,
        is_now: true,
    }];
    write_commit(&state, &planned)?;
    shard.now = next;
    Ok(true)
}

pub fn commit_pending(shard: &mut Shard) -> Result<CommitOutcome, Code> {
    let pending = shard.pending.take().ok_or(Code::CommitConflict)?;
    let mut now = pending.now;
    now.state_rev = now.state_rev.checked_add(1).ok_or(Code::CommitConflict)?;
    canonicalize(&mut now);
    let now_bytes = emit_now(&now)?.into_bytes();
    let tick_index = pending.tick_index;
    let log_path = format!("state/log/tick-{tick_index:04}.jsonl");
    let state = shard.root.join("state");
    let planned = plan_commit(
        &state,
        &now,
        now_bytes,
        Some((tick_index, pending.tick_jsonl.as_bytes())),
        &pending.memories,
        &pending.proposals,
    )?;
    write_commit(&state, &planned)?;
    shard.now = now;
    shard.queue.clear();
    Ok(CommitOutcome {
        state_rev: shard.now.state_rev,
        diff_summary: pending.paths,
        log_path,
    })
}

pub fn migrate_pack(root: &Path) -> Result<(), Code> {
    if !root.join("pack.yaml").is_file() || !root.join("state/now.yaml").is_file() {
        return Err(Code::NotInSpec);
    }
    let now_bytes = fs::read(root.join("state/now.yaml")).map_err(|_| Code::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Code::Schema)?;
    let mut now = load_now(now_text)?;
    let pack_rev_after = pack_rev(root)?;
    let pack_rev_before = now.pack_rev.clone();
    let state_rev_before = now.state_rev;
    let _lock = lock_pack(root)?;
    now.pack_rev = pack_rev_after;
    now.state_rev = now.state_rev.checked_add(1).ok_or(Code::CommitConflict)?;
    canonicalize(&mut now);
    let emitted = emit_now(&now)?.into_bytes();
    let state = root.join("state");
    let planned = plan_commit(&state, &now, emitted, None, &[], &[])?;
    write_commit(&state, &planned)?;
    let mut line = String::from("{\"op\":");
    push_json_string(&mut line, "migrate");
    line.push_str(",\"pack_rev_before\":");
    push_json_string(&mut line, &pack_rev_before);
    line.push_str(",\"pack_rev_after\":");
    push_json_string(&mut line, &now.pack_rev);
    line.push_str(",\"state_rev_before\":");
    line.push_str(&state_rev_before.to_string());
    line.push_str(",\"state_rev_after\":");
    line.push_str(&now.state_rev.to_string());
    line.push_str("}\n");
    append_studio(&state, &line)?;
    Ok(())
}

fn append_studio(state: &Path, line: &str) -> Result<(), Code> {
    let log_dir = state.join("log");
    fs::create_dir_all(&log_dir).map_err(|_| Code::CommitConflict)?;
    let path = log_dir.join("studio.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|_| Code::CommitConflict)?;
    file.write_all(line.as_bytes()).map_err(|_| Code::CommitConflict)?;
    file.sync_all().map_err(|_| Code::CommitConflict)?;
    fsync_dir(&log_dir)?;
    Ok(())
}

fn plan_commit(
    state: &Path,
    now: &Now,
    now_bytes: Vec<u8>,
    tick: Option<(u64, &[u8])>,
    memories: &[MemoryLine],
    proposals: &[ProposalDoc],
) -> Result<Vec<Planned>, Code> {
    let rumor_bytes = emit_rumors(now)?.into_bytes();
    let mut planned = Vec::new();
    planned.push(Planned {
        target: state.join("now.yaml"),
        bytes: now_bytes,
        is_now: true,
    });
    planned.push(Planned {
        target: state.join("rumors.yaml"),
        bytes: rumor_bytes,
        is_now: false,
    });
    if let Some((tick_index, tick_jsonl)) = tick {
        planned.push(Planned {
            target: state.join(format!("log/tick-{tick_index:04}.jsonl")),
            bytes: tick_jsonl.to_vec(),
            is_now: false,
        });
    }
    for (actor_id, body) in memory_bodies(state, memories)? {
        planned.push(Planned {
            target: state.join("memories").join(format!("{actor_id}.jsonl")),
            bytes: body.into_bytes(),
            is_now: false,
        });
    }
    for doc in proposals {
        planned.push(Planned {
            target: state.join("proposals").join(format!("{}.yaml", doc.id)),
            bytes: emit_proposal(doc)?.into_bytes(),
            is_now: false,
        });
    }
    Ok(planned)
}

struct Planned {
    target: PathBuf,
    bytes: Vec<u8>,
    is_now: bool,
}

fn write_commit(state: &Path, planned: &[Planned]) -> Result<(), Code> {
    let txn_dir = state.join(".txn");
    let prev = txn_dir.join("prev");
    fs::create_dir_all(&prev).map_err(|_| Code::CommitConflict)?;
    for item in planned {
        if item.target.is_file() {
            let rel = item
                .target
                .strip_prefix(state)
                .map_err(|_| Code::CommitConflict)?;
            let dest = prev.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|_| Code::CommitConflict)?;
            }
            fs::copy(&item.target, &dest).map_err(|_| Code::CommitConflict)?;
            fsync_file(&dest)?;
        }
    }
    fsync_dir(&prev)?;
    fsync_dir(&txn_dir)?;
    let complete = txn_dir.join("COMPLETE");
    {
        let file = File::create(&complete).map_err(|_| Code::CommitConflict)?;
        file.sync_all().map_err(|_| Code::CommitConflict)?;
    }
    fsync_dir(&txn_dir)?;

    let mut temps = Vec::new();
    for item in planned {
        let tmp = tmp_path(&item.target);
        if let Some(parent) = tmp.parent() {
            fs::create_dir_all(parent).map_err(|_| Code::CommitConflict)?;
        }
        {
            let mut file = File::create(&tmp).map_err(|_| Code::CommitConflict)?;
            file.write_all(&item.bytes).map_err(|_| Code::CommitConflict)?;
            file.sync_all().map_err(|_| Code::CommitConflict)?;
        }
        if let Some(parent) = item.target.parent() {
            fsync_dir(parent)?;
        }
        temps.push((tmp, item));
    }
    for (tmp, item) in &temps {
        if !item.is_now {
            fs::rename(tmp, &item.target).map_err(|_| Code::CommitConflict)?;
        }
    }
    for (tmp, item) in &temps {
        if item.is_now {
            fs::rename(tmp, &item.target).map_err(|_| Code::CommitConflict)?;
        }
    }
    fsync_dir(state)?;
    let _ = fs::remove_dir_all(&txn_dir);
    Ok(())
}

fn memory_bodies(state: &Path, lines: &[MemoryLine]) -> Result<BTreeMap<String, String>, Code> {
    let mut by_actor: BTreeMap<String, Vec<&MemoryLine>> = BTreeMap::new();
    for line in lines {
        by_actor.entry(line.actor_id.clone()).or_default().push(line);
    }
    let mut out = BTreeMap::new();
    for (actor, fresh) in by_actor {
        let path = state.join("memories").join(format!("{actor}.jsonl"));
        let mut body = if path.is_file() {
            fs::read_to_string(&path).map_err(|_| Code::CommitConflict)?
        } else {
            String::new()
        };
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        for line in fresh {
            body.push_str(&render_memory_line(line));
            body.push('\n');
        }
        out.insert(actor, body);
    }
    Ok(out)
}

fn render_memory_line(line: &MemoryLine) -> String {
    let mut out = String::from("{\"tick_index\":");
    out.push_str(&line.tick_index.to_string());
    out.push_str(",\"actor_id\":");
    push_json_string(&mut out, &line.actor_id);
    out.push_str(",\"intent_id\":");
    push_json_string(&mut out, &line.intent_id);
    out.push_str(",\"fact_ids\":");
    push_json_strings(&mut out, &line.fact_ids);
    out.push_str(",\"tags\":");
    push_json_strings(&mut out, &line.tags);
    out.push_str(",\"text\":");
    match &line.text {
        Some(text) => push_json_string(&mut out, text),
        None => out.push_str("null"),
    }
    out.push('}');
    out
}

fn push_json_strings(out: &mut String, values: &[String]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_json_string(out, value);
    }
    out.push(']');
}

fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other.is_control() => {
                out.push_str(&format!("\\u{:04x}", other as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

fn recover(root: &Path) -> Result<(), Code> {
    let state = root.join("state");
    let txn = state.join(".txn");
    if txn.is_dir() {
        if !txn.join("COMPLETE").is_file() {
            fs::remove_dir_all(&txn).map_err(|_| Code::CommitConflict)?;
        } else {
            let prev_now = txn.join("prev/now.yaml");
            if !prev_now.is_file() {
                return Err(Code::Schema);
            }
            let prev = load_now(&fs::read_to_string(&prev_now).map_err(|_| Code::Schema)?)?;
            let live = load_now(
                &fs::read_to_string(state.join("now.yaml")).map_err(|_| Code::NotInSpec)?,
            )?;
            if live.state_rev == prev.state_rev.saturating_add(1) && live.state_rev != prev.state_rev
            {
                fs::remove_dir_all(&txn).map_err(|_| Code::CommitConflict)?;
            } else if live.state_rev == prev.state_rev {
                restore_prev(&txn.join("prev"), &state)?;
                fs::remove_dir_all(&txn).map_err(|_| Code::CommitConflict)?;
            } else {
                return Err(Code::Schema);
            }
        }
    }

    let now = load_now(&fs::read_to_string(state.join("now.yaml")).map_err(|_| Code::NotInSpec)?)?;
    let log_dir = state.join("log");
    let mut live_ticks = BTreeSet::new();
    if log_dir.is_dir() {
        let mut entries = fs::read_dir(&log_dir)
            .map_err(|_| Code::Schema)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Code::Schema)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("tick-") || !name.ends_with(".jsonl") {
                continue;
            }
            let text = fs::read_to_string(&path).map_err(|_| Code::Schema)?;
            match (json_u64(&text, "tick_index"), json_u64(&text, "state_rev_after")) {
                (Some(tick_index), Some(rev)) if rev <= now.state_rev => {
                    live_ticks.insert(tick_index);
                }
                _ => {
                    fs::remove_file(&path).map_err(|_| Code::CommitConflict)?;
                }
            }
        }
    }

    let memories = state.join("memories");
    if memories.is_dir() {
        let entries = fs::read_dir(&memories)
            .map_err(|_| Code::Schema)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Code::Schema)?;
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let text = fs::read_to_string(&path).map_err(|_| Code::Schema)?;
            let mut kept = String::new();
            let mut changed = false;
            for line in text.lines() {
                if line.is_empty() {
                    continue;
                }
                match json_u64(line, "tick_index") {
                    Some(tick_index) if live_ticks.contains(&tick_index) => {
                        kept.push_str(line);
                        kept.push('\n');
                    }
                    _ => changed = true,
                }
            }
            if changed {
                atomic_write(&path, kept.as_bytes())?;
            }
        }
    }

    let proposals = state.join("proposals");
    if proposals.is_dir() {
        let entries = fs::read_dir(&proposals)
            .map_err(|_| Code::Schema)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Code::Schema)?;
        for entry in entries {
            let path = entry.path();
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default();
            if !now.proposals.iter().any(|id| id == stem) {
                fs::remove_file(&path).map_err(|_| Code::CommitConflict)?;
            }
        }
    }

    let rumors_path = state.join("rumors.yaml");
    if rumors_path.is_file() {
        let text = fs::read_to_string(&rumors_path).map_err(|_| Code::Schema)?;
        let parsed: Result<Vec<crate::model::Rumor>, _> = serde_saphyr::from_str(&text);
        let differs = match parsed {
            Ok(mut loaded) => {
                loaded.sort_by(|a, b| a.id.cmp(&b.id));
                let mut live = now.rumors.clone();
                live.sort_by(|a, b| a.id.cmp(&b.id));
                loaded != live
            }
            Err(_) => true,
        };
        if differs {
            atomic_write(&rumors_path, emit_rumors(&now)?.as_bytes())?;
        }
    }

    delete_tmp(&state);
    Ok(())
}

fn restore_prev(prev: &Path, state: &Path) -> Result<(), Code> {
    let mut files = Vec::new();
    collect_rel_files(prev, prev, &mut files)?;
    for rel in files {
        let from = prev.join(&rel);
        let to = state.join(&rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|_| Code::CommitConflict)?;
        }
        fs::copy(&from, &to).map_err(|_| Code::CommitConflict)?;
        fsync_file(&to)?;
    }
    fsync_dir(state)?;
    Ok(())
}

fn collect_rel_files(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) -> Result<(), Code> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(dir)
        .map_err(|_| Code::Schema)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Code::Schema)?;
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_rel_files(&path, root, out)?;
        } else {
            out.push(path.strip_prefix(root).map_err(|_| Code::Schema)?.to_path_buf());
        }
    }
    Ok(())
}

fn json_u64(text: &str, key: &str) -> Option<u64> {
    let pattern = format!("\"{key}\":");
    let index = text.find(&pattern)?;
    let rest = text[index + pattern.len()..].trim_start();
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), Code> {
    let tmp = tmp_path(target);
    if let Some(parent) = tmp.parent() {
        fs::create_dir_all(parent).map_err(|_| Code::CommitConflict)?;
    }
    {
        let mut file = File::create(&tmp).map_err(|_| Code::CommitConflict)?;
        file.write_all(bytes).map_err(|_| Code::CommitConflict)?;
        file.sync_all().map_err(|_| Code::CommitConflict)?;
    }
    fs::rename(&tmp, target).map_err(|_| Code::CommitConflict)?;
    if let Some(parent) = target.parent() {
        fsync_dir(parent)?;
    }
    Ok(())
}

fn delete_tmp(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name() == ".txn" {
                continue;
            }
            delete_tmp(&path);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".tmp"))
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn tmp_path(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .unwrap_or_default()
        .to_os_string();
    name.push(".tmp");
    target.with_file_name(name)
}

fn fsync_file(path: &Path) -> Result<(), Code> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| Code::CommitConflict)?;
    file.sync_all().map_err(|_| Code::CommitConflict)?;
    Ok(())
}

fn fsync_dir(path: &Path) -> Result<(), Code> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .map_err(|_| Code::CommitConflict)?;
        file.sync_all().map_err(|_| Code::CommitConflict)?;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let file = File::open(path).map_err(|_| Code::CommitConflict)?;
        file.sync_all().map_err(|_| Code::CommitConflict)?;
        Ok(())
    }
}
