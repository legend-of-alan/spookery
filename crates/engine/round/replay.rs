//! Walk the journal and check that the save matches.

use std::fs;
use std::path::Path;

use crate::canon::{check_save, migrate_save};
use crate::commit::atomic_publish;
use crate::diagnostic::reason_of;
use crate::effect::Effect;
use crate::journal::{self, hash_now, JournalEntry, MigrateRecord, RoundRecord};
use crate::model::{Now, Presence};
use crate::store::codec::{emit_effect, emit_save, load_save};
use crate::store::require_rules_rev;
use crate::world::World;
use crate::Reason;

use crate::round::dice::round_seed;
use crate::round::play::{play_round, RoundKind};

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
    pub round_index: Option<u64>,
    pub detail: String,
    pub replayed: Option<Vec<u8>>,
}

impl std::fmt::Display for ReplayDivergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.round_index {
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

/// Replay the journal. `from` is a directory whose `state/save.yaml` replaces the base.
/// `until` ignores records with a later `state_rev`, so a live server cannot race a verify.
pub fn from_journal(
    pack: &Path,
    from: Option<&Path>,
    until: Option<u64>,
) -> Result<ReplayReport, ReplayError> {
    if !pack.join("pack.yaml").is_file() {
        return Err(ReplayError::Reason(Reason::NotInSpec));
    }
    let records = load_entries(pack)?;
    let base_path = pack.join("state").join(journal::base_rel());
    let save_path = pack.join("state/save.yaml");
    let base_bytes = if let Some(from) = from {
        fs::read(from.join("state/save.yaml"))
            .map_err(|_| ReplayError::Reason(Reason::NotInSpec))?
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
    let mut world =
        World::open_for_replay(pack, load_base(&base_bytes)?).map_err(ReplayError::Reason)?;
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
        saved = read_now_bytes(&world)?;
    }
    Ok(ReplayReport {
        state_rev: world.now().state_rev,
        applied,
        now_yaml: saved,
    })
}

/// Open the pack at the journal tip. `save.yaml` stays the last checkpoint.
/// Records after that checkpoint are in the live bundle, so closed bundles stay unread.
pub(crate) fn resume(path: &Path) -> Result<World, Reason> {
    rebuild_snapshot(path).map_err(replay_reason)?;
    let mut world = World::open(path)?;
    // Open checked a save with nothing past it. A replayed tip is checked here.
    if replay_tail(&mut world)? {
        check_save(world.canon(), world.now()).map_err(reason_of)?;
    }
    Ok(world)
}

/// Migrate the journal tip. A save that the journal has not passed is migrated as it stands.
pub fn migrate_pack(path: &Path) -> Result<(), Reason> {
    let now = journal_tip(path)?;
    crate::commit::migrate_journal(path, now)
}

fn journal_tip(path: &Path) -> Result<Now, Reason> {
    let bytes = fs::read(path.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Reason::Schema)?;
    let file_now = load_save(text).map_err(reason_of)?;
    crate::store::recover::recover(path)?;
    let ahead = journal::read_live(path)
        .map_err(|_| Reason::Schema)?
        .iter()
        .any(|entry| {
            !matches!(entry, JournalEntry::Checkpoint(_))
                && entry.state_rev_after() > file_now.state_rev
        });
    if ahead {
        Ok(resume_for_migrate(path)?.now().clone())
    } else {
        Ok(file_now)
    }
}

/// The journal tip in memory and unchecked. Migrate checks it after adding its meters.
/// The caller runs crash recovery first.
pub(crate) fn resume_for_migrate(path: &Path) -> Result<World, Reason> {
    rebuild_snapshot(path).map_err(replay_reason)?;
    let save = fs::read(path.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let mut world = World::open_for_replay(path, load_base(&save).map_err(replay_reason)?)?;
    replay_tail(&mut world)?;
    Ok(world)
}

/// Replay the live records past the save. `true` when any record was replayed.
fn replay_tail(world: &mut World) -> Result<bool, Reason> {
    let entries =
        journal::read_live(world.root()).map_err(|fault| replay_reason(journal_fault(fault)))?;
    let base_rev = world.now().state_rev;
    let mut replayed = false;
    for entry in &entries {
        if entry.state_rev_after() <= base_rev {
            continue;
        }
        apply(world, entry).map_err(replay_reason)?;
        replayed = true;
    }
    // Seats are this process. The journal records who was present so the round
    // replays, and the save leaves them out.
    world.set_presence_inner(Presence {
        humans: 0,
        actors_present: Vec::new(),
    });
    crate::commit::settle_open_checkpoint(world)?;
    Ok(replayed)
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
            Some(mark.round_index),
            "state hash differed".to_string(),
            Some(report.now_yaml),
        )));
    }
    atomic_publish(&save, &report.now_yaml).map_err(ReplayError::Reason)?;
    Ok(())
}

/// Read the journal, replay it, and check checkpoints and `state/save.yaml`.
pub fn verify_pack(pack: &Path, from: Option<&Path>) -> Result<VerifyReport, VerifyError> {
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
        let genesis = read_genesis(pack)?;
        verify_save_file(pack, &records, &genesis)?;
        return Ok(VerifyReport {
            state_rev: state_rev_of(&genesis).map_err(VerifyError::Replay)?,
            applied: 0,
        });
    }
    let cap = records.iter().map(JournalEntry::state_rev_after).max();
    let report = match from_journal(pack, from, cap) {
        Ok(report) => report,
        Err(ReplayError::Diverged(diverged)) => {
            return Err(with_diff(diverged, &read_snapshot(pack)));
        }
        Err(err) => return Err(VerifyError::Replay(err)),
    };
    let verified = VerifyReport {
        state_rev: report.state_rev,
        applied: report.applied,
    };
    if from.is_some() {
        return Ok(verified);
    }
    if let Err(fault) = journal::check_chain(pack, &records) {
        return Err(VerifyError::Replay(journal_fault(fault)));
    }
    let genesis = read_genesis(pack)?;
    let bundles =
        journal::bundle_entries(pack).map_err(|fault| VerifyError::Replay(journal_fault(fault)))?;
    let starts = bundle_starts(pack, &genesis, &bundles)?;
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
    let mut tip = genesis.clone();
    for (index, end) in ends.into_iter().enumerate() {
        let end = end.unwrap_or(Err(ReplayError::Reason(Reason::Schema)));
        let (name, entries) = &bundles[index];
        tip = check_bundle_end(pack, name, entries, end)?;
    }
    if report.now_yaml != tip {
        return Err(VerifyError::Replay(ReplayError::Diverged(diverge(
            verified.state_rev,
            "save",
            None,
            "parallel replay differed from serial replay".to_string(),
            Some(tip),
        ))));
    }
    verify_save_file(pack, &records, &genesis)?;
    Ok(verified)
}

/// Check every chain hash, then replay the latest bundle from the snapshot it starts at.
pub fn verify_quick(pack: &Path) -> Result<VerifyReport, VerifyError> {
    let records = load_entries(pack).map_err(VerifyError::Replay)?;
    if let Err(fault) = journal::check_chain(pack, &records) {
        return Err(VerifyError::Replay(journal_fault(fault)));
    }
    let genesis = read_genesis(pack)?;
    let bundles =
        journal::bundle_entries(pack).map_err(|fault| VerifyError::Replay(journal_fault(fault)))?;
    let Some((start, entries)) = bundle_starts(pack, &genesis, &bundles)?.pop() else {
        verify_save_file(pack, &records, &genesis)?;
        return Ok(VerifyReport {
            state_rev: state_rev_of(&genesis).map_err(VerifyError::Replay)?,
            applied: 0,
        });
    };
    let (name, _) = bundles.last().expect("bundle");
    let end = check_bundle_end(pack, name, entries, replay_bundle(pack, &start, entries))?;
    verify_save_file(pack, &records, &genesis)?;
    Ok(VerifyReport {
        state_rev: state_rev_of(&end).map_err(VerifyError::Replay)?,
        applied: entries.iter().filter(|entry| entry.changes_state()).count() as u64,
    })
}

type BundleStart<'a> = (Vec<u8>, &'a [JournalEntry]);

fn bundle_starts<'a>(
    pack: &Path,
    genesis: &[u8],
    bundles: &'a [(String, Vec<JournalEntry>)],
) -> Result<Vec<BundleStart<'a>>, VerifyError> {
    let mut starts = Vec::with_capacity(bundles.len());
    let mut current = genesis.to_vec();
    for (index, (name, entries)) in bundles.iter().enumerate() {
        starts.push((current, entries.as_slice()));
        if index + 1 == bundles.len() {
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
        current = fs::read(&snap).map_err(|_| {
            VerifyError::Replay(ReplayError::Diverged(diverge(
                mark.state_rev,
                "checkpoint",
                Some(mark.round_index),
                format!("{name}: missing snapshot"),
                None,
            )))
        })?;
        if journal::hash_now(&current) != mark.state_hash {
            return Err(VerifyError::Replay(ReplayError::Diverged(diverge(
                mark.state_rev,
                "checkpoint",
                Some(mark.round_index),
                format!("{name}: state hash differed"),
                Some(current),
            ))));
        }
    }
    Ok(starts)
}

fn check_bundle_end(
    pack: &Path,
    name: &str,
    entries: &[JournalEntry],
    end: Result<Vec<u8>, ReplayError>,
) -> Result<Vec<u8>, VerifyError> {
    let end = end.map_err(|err| match err {
        ReplayError::Diverged(diverged) => with_diff(diverged, &read_snapshot(pack)),
        other => VerifyError::Replay(other),
    })?;
    if let Some(mark) = journal::latest_checkpoint(entries) {
        if journal::hash_now(&end) != mark.state_hash {
            return Err(VerifyError::Replay(ReplayError::Diverged(diverge(
                mark.state_rev,
                "checkpoint",
                Some(mark.round_index),
                format!("{name}: state hash differed"),
                Some(end),
            ))));
        }
    }
    Ok(end)
}

fn verify_save_file(
    pack: &Path,
    records: &[JournalEntry],
    genesis: &[u8],
) -> Result<(), VerifyError> {
    let live = fs::read(pack.join("state/save.yaml"))
        .map_err(|_| VerifyError::Replay(ReplayError::Reason(Reason::NotInSpec)))?;
    let now = load_base(&live).map_err(VerifyError::Replay)?;
    if records.is_empty() {
        let canonical = emit_save(&now)
            .map(|text| text.into_bytes())
            .map_err(|reason| VerifyError::Replay(ReplayError::Reason(reason)))?;
        if live != canonical {
            return Err(with_diff(
                diverge(
                    now.state_rev,
                    "save",
                    None,
                    Reason::HandEdit.token().to_string(),
                    Some(canonical),
                ),
                &live,
            ));
        }
    }
    if let Some(mark) = journal::latest_checkpoint(records) {
        if journal::hash_now(&live) != mark.state_hash {
            let snap = fs::read(
                pack.join("state/checkpoints")
                    .join(format!("{}.yaml", mark.state_rev)),
            )
            .ok();
            return Err(with_diff(
                diverge(
                    mark.state_rev,
                    "save",
                    Some(mark.round_index),
                    "state hash differed".to_string(),
                    snap,
                ),
                &live,
            ));
        }
    } else if live != genesis {
        return Err(with_diff(
            diverge(
                now.state_rev,
                "save",
                None,
                "state hash differed".to_string(),
                Some(genesis.to_vec()),
            ),
            &live,
        ));
    }
    Ok(())
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
    let mut world = World::open_for_replay(pack, load_base(start)?).map_err(ReplayError::Reason)?;
    let mut saved = start.to_vec();
    for entry in entries {
        apply(&mut world, entry)?;
        if entry.changes_state() {
            saved = read_now_bytes(&world)?;
        }
    }
    Ok(saved)
}

fn load_entries(pack: &Path) -> Result<Vec<JournalEntry>, ReplayError> {
    journal::read_entries(pack).map_err(journal_fault)
}

fn journal_fault(fault: crate::journal::JournalFault) -> ReplayError {
    if fault.reason == Reason::RulesRev {
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
                records[index].round_index(),
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
            entry.round_index(),
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
        JournalEntry::Round(record) => apply_recorded_round(world, record),
        JournalEntry::Migrate(record) => apply_migrate(world, record),
        JournalEntry::Checkpoint(record) => {
            let written = read_now_bytes(world)?;
            if hash_now(&written) != record.state_hash {
                return Err(ReplayError::Diverged(diverge(
                    record.state_rev,
                    "checkpoint",
                    Some(record.round_index),
                    "state hash differed".to_string(),
                    Some(written),
                )));
            }
            Ok(())
        }
    }
}

fn apply_recorded_round(world: &mut World, record: &RoundRecord) -> Result<(), ReplayError> {
    require_rules_rev(record.rules_rev).map_err(ReplayError::Reason)?;
    if world.now().round_index + 1 != record.round_index {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.round_index),
            "tick_index differed".to_string(),
            None,
        )));
    }
    let seed = round_seed(&world.now().shard_id, record.round_index);
    if record.seed != seed {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.round_index),
            "seed differed".to_string(),
            None,
        )));
    }
    let replayed = play_round(
        world,
        &record.last_tick_at,
        RoundKind::Replay {
            intents: &record.intents,
            presence: Presence {
                humans: record.presence_humans,
                actors_present: record.actors_present.clone(),
            },
        },
    )
    .map(|output| output.effects)
    .map_err(|code| {
        ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.round_index),
            code.token().to_string(),
            read_now_bytes(world).ok(),
        ))
    })?;
    let effect_diff = first_effect_difference(&record.effects, &replayed);
    if world.now().state_rev != record.state_rev_after {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.round_index),
            "state_rev differed".to_string(),
            read_now_bytes(world).ok(),
        )));
    }
    let written = read_now_bytes(world)?;
    if hash_now(&written) != record.now_hash {
        let detail = effect_diff.unwrap_or_else(|| "now_hash differed".to_string());
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.round_index),
            detail,
            Some(written),
        )));
    }
    if let Some(detail) = effect_diff {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "tick",
            Some(record.round_index),
            detail,
            None,
        )));
    }
    Ok(())
}

pub(crate) fn first_effect_difference(logged: &[Effect], replayed: &[Effect]) -> Option<String> {
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
    if world.now().pack_rev != record.pack_rev_before {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "migrate",
            None,
            "pack_rev differed".to_string(),
            None,
        )));
    }
    let next = migrate_save(
        world.now().clone(),
        &record.pack_rev_after,
        record.state_rev_after,
        &record.meters,
    );
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

fn diverge(
    state_rev: u64,
    kind: &str,
    round_index: Option<u64>,
    detail: String,
    replayed: Option<Vec<u8>>,
) -> ReplayDivergence {
    ReplayDivergence {
        state_rev,
        kind: kind.to_string(),
        round_index,
        detail,
        replayed,
    }
}

fn with_diff(diverged: ReplayDivergence, live: &[u8]) -> VerifyError {
    let diff = diverged.replayed.as_deref().and_then(|replayed| {
        let live_text = std::str::from_utf8(live).ok()?;
        let replayed_text = std::str::from_utf8(replayed).ok()?;
        crate::world::first_diff_line(live_text, replayed_text)
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{from_journal, verify_pack, verify_quick};
    use crate::round::play_pack_once;

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

    #[test]
    fn verify_and_quick_verify_reject_tampered_save_after_checkpoint() {
        let pack = temp_docks("replay-save-tamper");
        let pack_yaml = pack.join("pack.yaml");
        let mut cfg = fs::read_to_string(&pack_yaml).unwrap();
        cfg.push_str("\ncheckpoint_every: 2\n");
        fs::write(&pack_yaml, cfg).unwrap();
        play_pack_once(&pack).unwrap();
        play_pack_once(&pack).unwrap();
        let save = pack.join("state/save.yaml");
        let text = fs::read_to_string(&save).unwrap();
        let tampered = text.replacen("state_rev: 2", "state_rev: 99", 1);
        assert_ne!(tampered, text);
        fs::write(&save, tampered).unwrap();
        let full_err = verify_pack(&pack, None).unwrap_err().to_string();
        assert!(full_err.contains("state_rev 2 save"), "{full_err}");
        assert!(full_err.contains("save.yaml:"), "{full_err}");
        let quick_err = verify_quick(&pack).unwrap_err().to_string();
        assert!(quick_err.contains("state_rev 2 save"), "{quick_err}");
        assert!(quick_err.contains("save.yaml:"), "{quick_err}");
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
