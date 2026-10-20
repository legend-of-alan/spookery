//! Walk the journal and check that the save matches.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use spookery_kernel::api::canon::{bind_inventory, bind_meters, pack_rev};
use spookery_kernel::api::commit::atomic_publish;
use spookery_kernel::api::diagnostic::reason_of;
use spookery_kernel::api::effect::Effect;
use spookery_kernel::api::journal::{self, hash_now, JournalEntry, MigrateRecord, TickRecord};
use spookery_kernel::api::model::{Now, Presence};
use spookery_kernel::api::store::codec::{canonicalize, emit_effect, emit_save, load_save};
use spookery_kernel::api::store::require_rules_rev;
use spookery_kernel::api::world::World;
use spookery_kernel::api::Reason;

use crate::play::{play_recorded, RecordedRound};
use crate::seed::round_seed;

#[derive(Debug)]
pub struct ReplayReport {
    pub state_rev: u64,
    pub applied: u64,
    pub now_yaml: Vec<u8>,
}

#[derive(Debug)]
pub struct VerifyReport {
    pub state_rev: u64,
    pub applied: u64,
}

#[derive(Debug)]
pub struct ReplayDivergence {
    pub state_rev: u64,
    pub kind: String,
    pub tick_index: Option<u64>,
    pub detail: String,
    pub replayed: Option<Vec<u8>>,
}

impl std::fmt::Display for ReplayDivergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tick_index {
            Some(tick) => write!(
                f,
                "state_rev {} {} {}: {}",
                self.state_rev, self.kind, tick, self.detail
            ),
            None => write!(
                f,
                "state_rev {} {}: {}",
                self.state_rev, self.kind, self.detail
            ),
        }
    }
}

impl std::error::Error for ReplayDivergence {}

#[derive(Debug)]
pub enum ReplayError {
    Reason(Reason),
    Diverged(ReplayDivergence),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reason(reason) => write!(f, "{reason}"),
            Self::Diverged(diverged) => write!(f, "{diverged}"),
        }
    }
}

impl std::error::Error for ReplayError {}

#[derive(Debug)]
pub enum VerifyError {
    Replay(ReplayError),
    Mismatch {
        diverged: ReplayDivergence,
        line_no: u32,
        line: String,
    },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Replay(err) => write!(f, "{err}"),
            Self::Mismatch {
                diverged,
                line_no,
                line,
            } => write!(f, "{diverged}\nsave.yaml:{line_no}: {line}"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// One `save.yaml` after an applied journal record. The base save is not included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayStep {
    pub state_rev: u64,
    pub save_yaml: Vec<u8>,
}

/// Replay the journal. `from` is a directory whose `state/save.yaml` replaces the base.
/// `until` ignores records with a later `state_rev`, so a live server cannot race a verify.
pub fn from_journal(
    pack: &Path,
    from: Option<&Path>,
    until: Option<u64>,
) -> Result<ReplayReport, ReplayError> {
    replay_walking(pack, from, until, |_rev, _bytes| Ok(()))
}

/// Replay once, in one temp copy, and return `save.yaml` after each applied record.
/// An empty journal, or a journal with nothing past the base, returns an empty vec.
pub fn replay_states(pack: &Path) -> Result<Vec<ReplayStep>, ReplayError> {
    let mut steps = Vec::new();
    let _report = replay_walking(pack, None, None, |rev, bytes| {
        steps.push(ReplayStep {
            state_rev: rev,
            save_yaml: bytes.to_vec(),
        });
        Ok(())
    })?;
    Ok(steps)
}

fn replay_walking(
    pack: &Path,
    from: Option<&Path>,
    until: Option<u64>,
    mut on_step: impl FnMut(u64, &[u8]) -> Result<(), ReplayError>,
) -> Result<ReplayReport, ReplayError> {
    if !pack.join("pack.yaml").is_file() {
        return Err(ReplayError::Reason(Reason::NotInSpec));
    }
    let records = load_entries(pack)?;
    let base_path = pack.join("state").join(journal::base_rel());
    let save_path = pack.join("state/save.yaml");
    let base_bytes = if let Some(from) = from {
        read_from_now(from)?
    } else if base_path.is_file() {
        fs::read(&base_path).map_err(|_| ReplayError::Reason(Reason::Schema))?
    } else if save_path.is_file() {
        let now_yaml = fs::read(&save_path).map_err(|_| ReplayError::Reason(Reason::NotInSpec))?;
        if records.is_empty() {
            return Ok(ReplayReport {
                state_rev: state_rev_of(&now_yaml)?,
                applied: 0,
                now_yaml,
            });
        }
        now_yaml
    } else {
        return Err(ReplayError::Reason(Reason::NotInSpec));
    };
    let base_rev = state_rev_of(&base_bytes)?;
    let cap = until.unwrap_or_else(|| {
        records
            .iter()
            .map(JournalEntry::state_rev_after)
            .filter(|rev| until_allows(*rev, until))
            .max()
            .unwrap_or(base_rev)
    });
    check_sequence(&records, cap, base_rev, from.is_none(), &base_bytes)?;
    let mut applied = 0u64;
    let mut saved = Vec::new();
    let needs_apply = records.iter().any(|entry| {
        let rev = entry.state_rev_after();
        entry.changes_state() && rev > base_rev && rev <= cap
    });
    if !needs_apply {
        return Ok(ReplayReport {
            state_rev: base_rev,
            applied: 0,
            now_yaml: base_bytes,
        });
    }
    let temp = TempDir::new();
    copy_dir(pack, &temp.0)?;
    let _ = fs::remove_dir_all(temp.0.join("state/.txn"));
    fs::write(temp.0.join("state/save.yaml"), &base_bytes)
        .map_err(|_| ReplayError::Reason(Reason::Schema))?;
    let mut world = World::open_for_replay(&temp.0).map_err(ReplayError::Reason)?;
    for entry in records.iter() {
        let rev = entry.state_rev_after();
        if rev <= base_rev || rev > cap {
            continue;
        }
        apply(&mut world, entry)?;
        if !entry.changes_state() {
            continue;
        }
        applied += 1;
        let now_yaml = read_now_bytes(&world)?;
        let saved_rev = state_rev_of(&now_yaml)?;
        on_step(saved_rev, &now_yaml)?;
        saved = now_yaml;
    }
    Ok(ReplayReport {
        state_rev: state_rev_of(&saved)?,
        applied,
        now_yaml: saved,
    })
}

/// Open the pack at the journal tip. `save.yaml` stays the last checkpoint.
/// Records after that checkpoint are in the live bundle, so closed bundles stay unread.
pub fn resume(path: &Path) -> Result<World, Reason> {
    resume_from(path, false)
}

/// The tip, so migrate can store a new hash of files the host would refuse.
pub(crate) fn resume_for_migrate(path: &Path) -> Result<World, Reason> {
    resume_from(path, true)
}

fn resume_from(path: &Path, migrating: bool) -> Result<World, Reason> {
    rebuild_snapshot(path).map_err(replay_reason)?;
    let mut world = if migrating {
        World::open_for_replay(path)?
    } else {
        World::open(path)?
    };
    let entries = journal::read_live(path).map_err(|fault| replay_reason(journal_fault(fault)))?;
    let base_rev = world.now().state_rev;
    for entry in &entries {
        if entry.state_rev_after() <= base_rev {
            continue;
        }
        apply(&mut world, entry).map_err(replay_reason)?;
    }
    // Seats are this process. The journal records who was present so the round
    // replays, and the save leaves them out.
    world.set_presence(Presence {
        humans: 0,
        actors_present: Vec::new(),
    });
    spookery_kernel::api::commit::settle_open_checkpoint(&world)?;
    Ok(world)
}

fn replay_reason(err: ReplayError) -> Reason {
    match err {
        ReplayError::Reason(reason) => reason,
        ReplayError::Diverged(_) => Reason::Schema,
    }
}

fn rebuild_snapshot(path: &Path) -> Result<(), ReplayError> {
    let Some(mark) = journal::read_head(path).map_err(journal_fault)? else {
        return Ok(());
    };
    let save = path.join("state/save.yaml");
    if save.is_file() {
        let bytes = fs::read(&save).map_err(|_| ReplayError::Reason(Reason::NotInSpec))?;
        if hash_now(&bytes) == mark.state_hash {
            return Ok(());
        }
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if let Ok(now) = load_save(text) {
                if emit_save(&now)
                    .map(|emitted| emitted.into_bytes())
                    .ok()
                    .as_deref()
                    != Some(bytes.as_slice())
                {
                    return Ok(());
                }
            }
        }
    }
    let report = from_journal(path, None, Some(mark.state_rev))?;
    if hash_now(&report.now_yaml) != mark.state_hash {
        return Err(ReplayError::Diverged(diverge(
            mark.state_rev,
            "checkpoint",
            Some(mark.tick_index),
            "state hash differed".to_string(),
            Some(report.now_yaml),
        )));
    }
    atomic_publish(&save, &report.now_yaml).map_err(ReplayError::Reason)?;
    Ok(())
}

/// Read the journal and replay it. A stale `save.yaml` is not the tip.
pub fn verify_pack(pack: &Path, from: Option<&Path>) -> Result<VerifyReport, VerifyError> {
    verify_serial(pack, from).map(|(report, _yaml)| report)
}

/// Check every chain hash, then replay the latest bundle from the snapshot it starts at.
pub fn verify_quick(pack: &Path) -> Result<VerifyReport, VerifyError> {
    let records = load_entries(pack).map_err(VerifyError::Replay)?;
    if let Err(fault) = journal::check_chain(pack, &records) {
        return Err(VerifyError::Replay(journal_fault(fault)));
    }
    let bundles =
        journal::bundle_entries(pack).map_err(|fault| VerifyError::Replay(journal_fault(fault)))?;
    if bundles.is_empty() {
        let live = fs::read(pack.join("state/save.yaml"))
            .map_err(|_| VerifyError::Replay(ReplayError::Reason(Reason::NotInSpec)))?;
        return Ok(VerifyReport {
            state_rev: state_rev_of(&live).map_err(VerifyError::Replay)?,
            applied: 0,
        });
    }
    let mut start = read_genesis(pack)?;
    let last = bundles.len() - 1;
    for (index, (name, entries)) in bundles.iter().enumerate() {
        if index == last {
            break;
        }
        let Some(mark) = journal::latest_checkpoint(entries) else {
            return Err(VerifyError::Replay(ReplayError::Diverged(diverge(
                0,
                "checkpoint",
                None,
                format!("{name}: a closed bundle has no checkpoint"),
                None,
            ))));
        };
        let snap = pack
            .join("state/checkpoints")
            .join(format!("{}.yaml", mark.state_rev));
        start = fs::read(&snap).map_err(|_| {
            VerifyError::Replay(ReplayError::Diverged(diverge(
                mark.state_rev,
                "checkpoint",
                Some(mark.tick_index),
                format!("{name}: missing snapshot"),
                None,
            )))
        })?;
        if journal::hash_now(&start) != mark.state_hash {
            return Err(VerifyError::Replay(ReplayError::Diverged(diverge(
                mark.state_rev,
                "checkpoint",
                Some(mark.tick_index),
                format!("{name}: state hash differed"),
                Some(start),
            ))));
        }
    }
    let (name, entries) = &bundles[last];
    let end = replay_bundle(pack, &start, entries).map_err(|err| match err {
        ReplayError::Diverged(diverged) => with_diff(diverged, &read_snapshot(pack)),
        other => VerifyError::Replay(other),
    })?;
    if let Some(mark) = journal::latest_checkpoint(entries) {
        if journal::hash_now(&end) != mark.state_hash {
            return Err(VerifyError::Replay(ReplayError::Diverged(diverge(
                mark.state_rev,
                "checkpoint",
                Some(mark.tick_index),
                format!("{name}: state hash differed"),
                Some(end),
            ))));
        }
    }
    Ok(VerifyReport {
        state_rev: state_rev_of(&end).map_err(VerifyError::Replay)?,
        applied: entries.iter().filter(|entry| entry.changes_state()).count() as u64,
    })
}

/// Replay every bundle from its checkpoint snapshot, at the same time.
/// The tip matches one serial replay.
pub fn verify_parallel(pack: &Path) -> Result<VerifyReport, VerifyError> {
    let (serial, serial_yaml) = verify_serial(pack, None)?;
    let bundles =
        journal::bundle_entries(pack).map_err(|fault| VerifyError::Replay(journal_fault(fault)))?;
    if bundles.is_empty() {
        return Ok(serial);
    }
    let mut starts = Vec::new();
    let genesis = read_genesis(pack)?;
    let mut previous: Option<Vec<u8>> = None;
    for (index, (name, entries)) in bundles.iter().enumerate() {
        if index + 1 < bundles.len() && journal::latest_checkpoint(entries).is_none() {
            return Err(VerifyError::Replay(ReplayError::Diverged(diverge(
                0,
                "checkpoint",
                None,
                format!("{name}: a closed bundle has no checkpoint"),
                None,
            ))));
        }
        let start = previous.clone().unwrap_or_else(|| genesis.clone());
        starts.push((start, entries));
        if let Some(mark) = journal::latest_checkpoint(entries) {
            let snap = pack
                .join("state/checkpoints")
                .join(format!("{}.yaml", mark.state_rev));
            previous = Some(fs::read(&snap).map_err(|_| {
                VerifyError::Replay(ReplayError::Diverged(diverge(
                    mark.state_rev,
                    "checkpoint",
                    Some(mark.tick_index),
                    format!("{name}: missing snapshot"),
                    None,
                )))
            })?);
        }
    }
    let mut ends: Vec<Option<Result<Vec<u8>, ReplayError>>> =
        (0..starts.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for (index, (start, entries)) in starts.iter().enumerate() {
            handles.push(scope.spawn(move || (index, replay_bundle(pack, start, entries))));
        }
        for handle in handles {
            let (index, end) = handle.join().expect("replay");
            ends[index] = Some(end);
        }
    });
    let mut tip = genesis;
    for (index, end) in ends.into_iter().enumerate() {
        let end = end.unwrap_or(Err(ReplayError::Reason(Reason::Schema)));
        let end = end.map_err(|err| match err {
            ReplayError::Diverged(diverged) => with_diff(diverged, &read_snapshot(pack)),
            other => VerifyError::Replay(other),
        })?;
        let (name, entries) = &bundles[index];
        if let Some(mark) = journal::latest_checkpoint(entries) {
            if journal::hash_now(&end) != mark.state_hash {
                return Err(VerifyError::Replay(ReplayError::Diverged(diverge(
                    mark.state_rev,
                    "checkpoint",
                    Some(mark.tick_index),
                    format!("{name}: state hash differed"),
                    Some(end),
                ))));
            }
        }
        tip = end;
    }
    if serial_yaml != tip {
        return Err(VerifyError::Replay(ReplayError::Diverged(diverge(
            serial.state_rev,
            "save",
            None,
            "parallel replay differed from serial replay".to_string(),
            Some(tip),
        ))));
    }
    Ok(serial)
}

fn verify_serial(pack: &Path, from: Option<&Path>) -> Result<(VerifyReport, Vec<u8>), VerifyError> {
    if !pack.join("pack.yaml").is_file() {
        return Err(VerifyError::Replay(ReplayError::Reason(Reason::NotInSpec)));
    }
    if !pack.join("state/save.yaml").is_file()
        && !pack.join("state").join(journal::base_rel()).is_file()
    {
        return Err(VerifyError::Replay(ReplayError::Reason(Reason::NotInSpec)));
    }
    let records = load_entries(pack).map_err(VerifyError::Replay)?;
    if records.is_empty() && from.is_none() {
        let live = fs::read(pack.join("state/save.yaml"))
            .map_err(|_| VerifyError::Replay(ReplayError::Reason(Reason::NotInSpec)))?;
        let rev = state_rev_of(&live).map_err(VerifyError::Replay)?;
        return Ok((
            VerifyReport {
                state_rev: rev,
                applied: 0,
            },
            live,
        ));
    }
    let cap = records.iter().map(JournalEntry::state_rev_after).max();
    let report = match from_journal(pack, from, cap) {
        Ok(report) => report,
        Err(ReplayError::Diverged(diverged)) => {
            return Err(with_diff(diverged, &read_snapshot(pack)));
        }
        Err(err) => return Err(VerifyError::Replay(err)),
    };
    if from.is_none() {
        if let Err(fault) = journal::check_chain(pack, &records) {
            return Err(VerifyError::Replay(journal_fault(fault)));
        }
    }
    Ok((
        VerifyReport {
            state_rev: report.state_rev,
            applied: report.applied,
        },
        report.now_yaml,
    ))
}

fn read_snapshot(pack: &Path) -> Vec<u8> {
    fs::read(pack.join("state/save.yaml"))
        .or_else(|_| fs::read(pack.join("state").join(journal::base_rel())))
        .unwrap_or_default()
}

fn read_genesis(pack: &Path) -> Result<Vec<u8>, VerifyError> {
    let base = pack.join("state").join(journal::base_rel());
    if base.is_file() {
        return fs::read(&base)
            .map_err(|_| VerifyError::Replay(ReplayError::Reason(Reason::Schema)));
    }
    fs::read(pack.join("state/save.yaml"))
        .map_err(|_| VerifyError::Replay(ReplayError::Reason(Reason::NotInSpec)))
}

fn replay_bundle(
    pack: &Path,
    start: &[u8],
    entries: &[JournalEntry],
) -> Result<Vec<u8>, ReplayError> {
    let temp = TempDir::new();
    copy_dir(pack, &temp.0)?;
    fs::write(temp.0.join("state/save.yaml"), start)
        .map_err(|_| ReplayError::Reason(Reason::Schema))?;
    let mut world = World::open_for_replay(&temp.0).map_err(ReplayError::Reason)?;
    let mut saved = start.to_vec();
    for entry in entries {
        apply(&mut world, entry)?;
        if entry.changes_state() {
            saved = read_now_bytes(&world)?;
        }
    }
    Ok(saved)
}

fn until_allows(rev: u64, until: Option<u64>) -> bool {
    until.is_none_or(|cap| rev <= cap)
}

fn load_entries(pack: &Path) -> Result<Vec<JournalEntry>, ReplayError> {
    journal::read_entries(pack).map_err(journal_fault)
}

fn journal_fault(fault: spookery_kernel::api::journal::JournalFault) -> ReplayError {
    if fault.detail.contains("RULES_REV") {
        return ReplayError::Reason(Reason::RulesRev);
    }
    ReplayError::Diverged(diverge(
        0,
        "journal",
        None,
        format!("{}: {}", fault.file, fault.detail),
        None,
    ))
}

fn check_sequence(
    records: &[JournalEntry],
    cap: u64,
    base_rev: u64,
    engine_base: bool,
    base_bytes: &[u8],
) -> Result<(), ReplayError> {
    let mut marks: Vec<(u64, u64, usize)> = Vec::new();
    for (index, entry) in records.iter().enumerate() {
        if !entry.changes_state() {
            continue;
        }
        let rev = entry.state_rev_after();
        if rev > cap {
            continue;
        }
        marks.push((rev, entry.state_rev_before(), index));
    }
    marks.sort_by_key(|mark| mark.0);
    let mut expected = 1u64;
    for (rev, before, index) in marks {
        if rev != expected {
            return Err(ReplayError::Diverged(diverge(
                expected,
                "missing",
                None,
                "no journal record".to_string(),
                Some(base_bytes.to_vec()),
            )));
        }
        if before.saturating_add(1) != rev {
            return Err(ReplayError::Diverged(diverge(
                rev,
                records[index].kind_name(),
                records[index].tick_index(),
                "state_rev differed".to_string(),
                Some(base_bytes.to_vec()),
            )));
        }
        if engine_base && rev == base_rev {
            check_mark(&records[index], base_bytes)?;
        }
        expected += 1;
    }
    if cap > 0 && expected.saturating_sub(1) != cap {
        return Err(ReplayError::Diverged(diverge(
            expected,
            "missing",
            None,
            "no journal record".to_string(),
            Some(base_bytes.to_vec()),
        )));
    }
    Ok(())
}

fn check_mark(entry: &JournalEntry, base_bytes: &[u8]) -> Result<(), ReplayError> {
    let JournalEntry::Migrate(record) = entry else {
        return Err(ReplayError::Diverged(diverge(
            entry.state_rev_after(),
            entry.kind_name(),
            entry.tick_index(),
            "base mark is not a migrate".to_string(),
            Some(base_bytes.to_vec()),
        )));
    };
    if record.now_hash != hash_now(base_bytes) {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "migrate",
            None,
            "now_hash differed".to_string(),
            Some(base_bytes.to_vec()),
        )));
    }
    let base_now = load_base(base_bytes)?;
    if record.pack_rev_after != base_now.pack_rev {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "migrate",
            None,
            "pack_rev differed".to_string(),
            Some(base_bytes.to_vec()),
        )));
    }
    Ok(())
}

fn apply(world: &mut World, entry: &JournalEntry) -> Result<(), ReplayError> {
    match entry {
        JournalEntry::Tick(record) => apply_recorded_round(world, record),
        JournalEntry::Migrate(record) => apply_migrate(world, record),
        JournalEntry::Checkpoint(record) => {
            let written = read_now_bytes(world)?;
            if hash_now(&written) != record.state_hash {
                return Err(ReplayError::Diverged(diverge(
                    record.state_rev,
                    "checkpoint",
                    Some(record.tick_index),
                    "state hash differed".to_string(),
                    Some(written),
                )));
            }
            Ok(())
        }
    }
}

fn apply_recorded_round(world: &mut World, record: &TickRecord) -> Result<(), ReplayError> {
    require_rules_rev(record.rules_rev).map_err(ReplayError::Reason)?;
    if world.now().tick_index + 1 != record.tick_index {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.tick_index),
            "tick_index differed".to_string(),
            None,
        )));
    }
    let seed = round_seed(&world.now().shard_id, record.tick_index);
    if record.seed != seed {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.tick_index),
            "seed differed".to_string(),
            None,
        )));
    }
    let wall = record.last_tick_at.clone();
    let replayed = play_recorded(
        world,
        &wall,
        RecordedRound {
            intents: record.intents.clone(),
            presence_humans: record.presence_humans,
            actors_present: record.actors_present.clone(),
        },
    )
    .map_err(|code| {
        ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.tick_index),
            code.token().to_string(),
            read_now(world),
        ))
    })?;
    let effect_diff = first_effect_difference(&record.effects, &replayed);
    if world.now().state_rev != record.state_rev_after {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.tick_index),
            "state_rev differed".to_string(),
            read_now(world),
        )));
    }
    let written = read_now_bytes(world)?;
    if hash_now(&written) != record.now_hash {
        let detail = effect_diff.unwrap_or_else(|| "now_hash differed".to_string());
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.tick_index),
            detail,
            Some(written),
        )));
    }
    if let Some(detail) = effect_diff {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.tick_index),
            detail,
            None,
        )));
    }
    Ok(())
}

fn first_effect_difference(logged: &[Effect], replayed: &[Effect]) -> Option<String> {
    let count = logged.len().max(replayed.len());
    for index in 0..count {
        let left = logged.get(index);
        let right = replayed.get(index);
        if left == right {
            continue;
        }
        return Some(format!(
            "effect {index} differed: logged {} replayed {}",
            effect_text(left),
            effect_text(right)
        ));
    }
    None
}

fn effect_text(effect: Option<&Effect>) -> String {
    match effect {
        Some(effect) => emit_effect(effect),
        None => "none".to_string(),
    }
}

fn apply_migrate(world: &mut World, record: &MigrateRecord) -> Result<(), ReplayError> {
    let mut next = world.now().clone();
    if next.pack_rev != record.pack_rev_before {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "migrate",
            None,
            "pack_rev differed".to_string(),
            None,
        )));
    }
    bind_meters(world.canon(), &mut next).map_err(|diag| ReplayError::Reason(reason_of(diag)))?;
    bind_inventory(&next).map_err(|diag| ReplayError::Reason(reason_of(diag)))?;
    let computed = pack_rev(world.root()).map_err(ReplayError::Reason)?;
    if computed != record.pack_rev_after {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "migrate",
            None,
            "pack_rev differed".to_string(),
            None,
        )));
    }
    next.pack_rev = record.pack_rev_after.clone();
    next.state_rev = record.state_rev_after;
    canonicalize(&mut next);
    let bytes = emit_save(&next).map_err(ReplayError::Reason)?.into_bytes();
    if hash_now(&bytes) != record.now_hash {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "migrate",
            None,
            "now_hash differed".to_string(),
            Some(bytes),
        )));
    }
    world.install_replayed(next);
    Ok(())
}

fn read_from_now(from: &Path) -> Result<Vec<u8>, ReplayError> {
    let path = from.join("state/save.yaml");
    if !path.is_file() {
        return Err(ReplayError::Reason(Reason::NotInSpec));
    }
    fs::read(&path).map_err(|_| ReplayError::Reason(Reason::NotInSpec))
}

fn load_base(bytes: &[u8]) -> Result<Now, ReplayError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ReplayError::Reason(Reason::Schema))?;
    load_save(text).map_err(|diag| ReplayError::Reason(reason_of(diag)))
}

fn state_rev_of(bytes: &[u8]) -> Result<u64, ReplayError> {
    Ok(load_base(bytes)?.state_rev)
}

fn read_now_bytes(world: &World) -> Result<Vec<u8>, ReplayError> {
    emit_save(world.now())
        .map(|text| text.into_bytes())
        .map_err(ReplayError::Reason)
}

fn read_now(world: &World) -> Option<Vec<u8>> {
    read_now_bytes(world).ok()
}

fn diverge(
    state_rev: u64,
    kind: &str,
    tick_index: Option<u64>,
    detail: String,
    replayed: Option<Vec<u8>>,
) -> ReplayDivergence {
    ReplayDivergence {
        state_rev,
        kind: kind.to_string(),
        tick_index,
        detail,
        replayed,
    }
}

fn with_diff(diverged: ReplayDivergence, live: &[u8]) -> VerifyError {
    let diff = diverged.replayed.as_deref().and_then(|replayed| {
        let live_text = std::str::from_utf8(live).ok()?;
        let replayed_text = std::str::from_utf8(replayed).ok()?;
        first_diff_line(live_text, replayed_text)
    });
    match diff {
        Some((line_no, line)) => VerifyError::Mismatch {
            diverged,
            line_no,
            line,
        },
        None => VerifyError::Replay(ReplayError::Diverged(diverged)),
    }
}

fn first_diff_line(live: &str, replayed: &str) -> Option<(u32, String)> {
    for (index, (left, right)) in live.lines().zip(replayed.lines()).enumerate() {
        if left != right {
            return Some(((index + 1) as u32, left.to_string()));
        }
    }
    let live_count = live.lines().count();
    let replay_count = replayed.lines().count();
    if live_count != replay_count {
        let number = live_count.min(replay_count) + 1;
        let line = live
            .lines()
            .nth(number - 1)
            .or_else(|| replayed.lines().nth(number - 1))
            .unwrap_or("")
            .to_string();
        return Some((number as u32, line));
    }
    if live.as_bytes() != replayed.as_bytes() {
        return Some((1, live.lines().next().unwrap_or("").to_string()));
    }
    None
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("spook-replay-{}-{n}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), ReplayError> {
    fs::create_dir_all(to).map_err(|_| ReplayError::Reason(Reason::Schema))?;
    for entry in fs::read_dir(from).map_err(|_| ReplayError::Reason(Reason::Schema))? {
        let entry = entry.map_err(|_| ReplayError::Reason(Reason::Schema))?;
        let dest = to.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|_| ReplayError::Reason(Reason::Schema))?;
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest).map_err(|_| ReplayError::Reason(Reason::Schema))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{from_journal, replay_states, verify_pack};
    use crate::play_pack_once;

    #[test]
    fn two_replays_in_one_process_do_not_collide() {
        let pack = temp_docks("replay-collide");
        play_pack_once(&pack).unwrap();
        let left = from_journal(&pack, None, None).unwrap();
        let right = from_journal(&pack, None, None).unwrap();
        assert_eq!(left.now_yaml, right.now_yaml);
        assert_eq!(left.applied, 1);
        assert_eq!(right.applied, 1);
        assert_eq!(left.state_rev, 1);
        let _ = fs::remove_dir_all(&pack);
    }

    #[test]
    fn one_replay_returns_each_applied_save() {
        let pack = temp_docks("replay-states");
        play_pack_once(&pack).unwrap();
        play_pack_once(&pack).unwrap();
        let steps = replay_states(&pack).unwrap();
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert_eq!(steps[0].state_rev, 1);
        assert_eq!(steps[1].state_rev, 2);
        assert_ne!(steps[0].save_yaml, steps[1].save_yaml);
        let final_report = from_journal(&pack, None, None).unwrap();
        assert_eq!(steps[1].save_yaml, final_report.now_yaml);
        assert_eq!(final_report.applied, 2);
        let _ = fs::remove_dir_all(&pack);
    }

    #[test]
    fn verify_names_a_tick_whose_hash_no_longer_matches() {
        let pack = temp_docks("replay-tamper");
        play_pack_once(&pack).unwrap();
        let path = pack.join("state/journal/bundle-0000.jsonl");
        let text = fs::read_to_string(&path).unwrap();
        let swapped = text.replacen("\"now_hash\":\"blake2s:", "\"now_hash\":\"blake2s:00", 1);
        assert_ne!(swapped, text);
        fs::write(&path, swapped).unwrap();
        let err = verify_pack(&pack, None).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("state_rev 1"), "{message}");
        assert!(message.contains("tick 1"), "{message}");
        assert!(message.contains("now_hash differed"), "{message}");
        let _ = fs::remove_dir_all(&pack);
    }

    fn temp_docks(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "spook-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        copy_tree(&source, &root);
        root
    }

    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &dest);
            } else {
                fs::copy(entry.path(), dest).unwrap();
            }
        }
    }
}
