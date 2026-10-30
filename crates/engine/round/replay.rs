//! Walk the journal and rebuild the state it records.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::canon::{check_save, load_canon, migrate_save};
use crate::commit::write_atomic;
use crate::diagnostic::first_reason;
use crate::effect::Effect;
use crate::journal::{self, blake2s_tagged, MigrateRecord, Record, RoundRecord};
use crate::lock::{lock_pack, lock_write, PackGuard};
use crate::model::{Presence, State};
use crate::store::codec::{emit_effect, emit_save, parse_save};
use crate::store::require_rules_rev;
use crate::world::World;
use crate::Reason;

use crate::round::dice::round_seed;
use crate::round::play::replay_round;

#[derive(Debug)]
pub struct Report {
    pub state_rev: u64,
    pub applied: u64,
    pub now_yaml: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub enum DivergenceKind {
    Round,
    Migrate,
    Checkpoint,
    Save,
    Journal,
    Missing,
}

impl DivergenceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Round => "round",
            Self::Migrate => "migrate",
            Self::Checkpoint => "checkpoint",
            Self::Save => "save",
            Self::Journal => "journal",
            Self::Missing => "missing",
        }
    }
}

impl From<&Record> for DivergenceKind {
    fn from(entry: &Record) -> Self {
        match entry {
            Record::Round(_) => Self::Round,
            Record::Migrate(_) => Self::Migrate,
            Record::Checkpoint(_) => Self::Checkpoint,
        }
    }
}

#[derive(Debug)]
pub struct Divergence {
    pub state_rev: u64,
    pub kind: DivergenceKind,
    pub round_index: Option<u64>,
    pub detail: String,
    pub replayed: Option<Vec<u8>>,
    /// The first `save.yaml` line that differs from the replayed bytes.
    pub save_line: Option<(u32, String)>,
}

impl Divergence {
    pub(crate) fn new(
        kind: DivergenceKind,
        state_rev: u64,
        round_index: Option<u64>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            state_rev,
            kind,
            round_index,
            detail: detail.into(),
            replayed: None,
            save_line: None,
        }
    }

    pub(crate) fn replayed(self, bytes: Vec<u8>) -> Self {
        Self {
            replayed: Some(bytes),
            ..self
        }
    }
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "state_rev {} {}", self.state_rev, self.kind.as_str())?;
        if let Some(round) = self.round_index {
            write!(f, " {round}")?;
        }
        write!(f, ": {}", self.detail)?;
        if let Some((line_no, line)) = &self.save_line {
            write!(f, "\nsave.yaml:{line_no}: {line}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Divergence {}

#[derive(Debug)]
pub enum Error {
    Reason(Reason),
    Diverged(Divergence),
}

impl From<Divergence> for Error {
    fn from(diverged: Divergence) -> Self {
        Self::Diverged(diverged)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reason(reason) => write!(f, "{reason}"),
            Self::Diverged(diverged) => write!(f, "{diverged}"),
        }
    }
}

impl std::error::Error for Error {}

/// A round record that did not replay as written.
fn round_diverged(record: &RoundRecord, detail: impl Into<String>) -> Divergence {
    Divergence::new(
        DivergenceKind::Round,
        record.state_rev_after,
        Some(record.round_index),
        detail,
    )
}

/// A checkpoint whose hash no longer matches.
pub(crate) fn checkpoint_diverged(
    state_rev: u64,
    round_index: u64,
    detail: impl Into<String>,
) -> Divergence {
    Divergence::new(
        DivergenceKind::Checkpoint,
        state_rev,
        Some(round_index),
        detail,
    )
}

/// Replay the journal. `from` is a directory whose `state/save.yaml` replaces the base.
/// `until` ignores records with a later `state_rev`, so a live server cannot race a verify.
pub fn replay_journal(
    pack: &Path,
    from: Option<&Path>,
    until: Option<u64>,
) -> Result<Report, Error> {
    if !pack.join("pack.yaml").is_file() {
        return Err(Error::Reason(Reason::NotInSpec));
    }
    let records = journal::read_records(pack)?;
    let base_path = pack.join("state").join(journal::base_path());
    let save_path = pack.join("state/save.yaml");
    let base_bytes = if let Some(from) = from {
        fs::read(from.join("state/save.yaml")).map_err(|_| Error::Reason(Reason::NotInSpec))?
    } else if base_path.is_file() {
        fs::read(&base_path).map_err(|_| Error::Reason(Reason::Schema))?
    } else if save_path.is_file() {
        let now_yaml = fs::read(&save_path).map_err(|_| Error::Reason(Reason::NotInSpec))?;
        if records.is_empty() {
            return Ok(Report {
                state_rev: state_rev_of(&now_yaml)?,
                applied: 0,
                now_yaml,
            });
        }
        now_yaml
    } else {
        return Err(Error::Reason(Reason::NotInSpec));
    };
    let base_rev = state_rev_of(&base_bytes)?;
    let cap = until.unwrap_or_else(|| {
        records
            .iter()
            .map(Record::state_rev_after)
            .max()
            .unwrap_or(base_rev)
    });
    check_sequence(&records, cap, base_rev, from.is_none(), &base_bytes)?;
    let mut applied = 0u64;
    let needs_apply = records.iter().any(|entry| {
        let rev = entry.state_rev_after();
        entry.changes_state() && rev > base_rev && rev <= cap
    });
    if !needs_apply {
        return Ok(Report {
            state_rev: base_rev,
            applied: 0,
            now_yaml: base_bytes,
        });
    }
    let mut world = open_for_replay(pack, &base_bytes)?;
    for entry in records.iter() {
        let rev = entry.state_rev_after();
        if rev <= base_rev || rev > cap {
            continue;
        }
        replay_record(&mut world, entry)?;
        if entry.changes_state() {
            applied += 1;
        }
    }
    Ok(Report {
        state_rev: world.state().state_rev,
        applied,
        now_yaml: emit_save_bytes(&world)?,
    })
}

/// Open the pack at the journal tip. `save.yaml` stays the last checkpoint.
/// Records after that checkpoint are in the live bundle, so closed bundles stay unread.
/// The caller holds the pack lock, so a crash is repaired and the checkpoint settled.
pub(crate) fn open_at_tip(path: &Path, lock: &PackGuard) -> Result<World<()>, Reason> {
    rebuild_save(path, lock).map_err(Error::into_reason)?;
    let world = peek_at_tip(path)?;
    crate::commit::finish_live_checkpoint(&world, lock)?;
    Ok(world)
}

/// The journal tip in memory. Writes nothing.
pub(crate) fn peek_at_tip(path: &Path) -> Result<World<()>, Reason> {
    let mut world = World::open(path)?;
    // Open checked a save with nothing past it. A replayed tip is checked here.
    if replay_tail(&mut world)? {
        check_save(world.canon(), world.state()).map_err(|diags| first_reason(&diags))?;
    }
    Ok(world)
}

/// Migrate the journal tip. A save that the journal has not passed is migrated as it stands.
pub fn migrate_pack(path: &Path) -> Result<(), Reason> {
    let lock = lock_pack(path)?;
    let (now, edited) = load_migrate_tip(path, &lock)?;
    crate::commit::commit_migrate(path, now, edited, &lock)
}

/// The state to migrate, and whether it is a hand edit of the checkpoint head.
/// A hand edit with live rounds past it is refused, because the tip does not hold it.
fn load_migrate_tip(path: &Path, lock: &PackGuard) -> Result<(State, bool), Reason> {
    let bytes = fs::read(path.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Reason::Schema)?;
    let file_now = parse_save(text).map_err(|diags| first_reason(&diags))?;
    // A save from before the head is a checkpoint that did not land, not an edit.
    let edited = journal::read_head(path)
        .map_err(|fault| fault.reason)?
        .is_some_and(|head| {
            head.state_rev == file_now.state_rev && head.state_hash != blake2s_tagged(&bytes)
        });
    let ahead = journal::read_live_records(path)
        .map_err(|_| Reason::Schema)?
        .iter()
        .any(|entry| {
            !matches!(entry, Record::Checkpoint(_)) && entry.state_rev_after() > file_now.state_rev
        });
    match (ahead, edited) {
        (true, true) => Err(Reason::HandEdit),
        (true, false) => Ok((replay_tip_unchecked(path, lock)?.state().clone(), false)),
        (false, _) => Ok((file_now, edited)),
    }
}

/// The journal tip in memory and unchecked. Migrate checks it after adding its meters.
/// The caller holds the pack lock.
fn replay_tip_unchecked(path: &Path, lock: &PackGuard) -> Result<World<()>, Reason> {
    rebuild_save(path, lock).map_err(Error::into_reason)?;
    let save = fs::read(path.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let mut world = open_for_replay(path, &save).map_err(Error::into_reason)?;
    replay_tail(&mut world)?;
    crate::commit::finish_live_checkpoint(&world, lock)?;
    Ok(world)
}

/// A replay world at `base` with the pack's canon.
pub(crate) fn open_for_replay(pack: &Path, base: &[u8]) -> Result<World<()>, Error> {
    let now = load_save_bytes(base)?;
    let canon = load_canon(pack).map_err(|diags| Error::Reason(first_reason(&diags)))?;
    Ok(World::from_parts(pack, Arc::new(canon), now))
}

/// Replay the live records past the save. `true` when any record was replayed.
fn replay_tail(world: &mut World<()>) -> Result<bool, Reason> {
    let entries = journal::read_live_records(world.root())
        .map_err(|fault| Error::from(fault).into_reason())?;
    let base_rev = world.state().state_rev;
    let mut replayed = false;
    for entry in &entries {
        if entry.state_rev_after() <= base_rev {
            continue;
        }
        replay_record(world, entry).map_err(Error::into_reason)?;
        replayed = true;
    }
    Ok(replayed)
}

impl Error {
    fn into_reason(self) -> Reason {
        match self {
            Error::Reason(reason) => reason,
            Error::Diverged(_) => Reason::Schema,
        }
    }
}

fn rebuild_save(path: &Path, lock: &PackGuard) -> Result<(), Error> {
    let Some(mark) = journal::read_head(path)? else {
        return Ok(());
    };
    let save = path.join("state/save.yaml");
    if save.is_file() {
        let bytes = fs::read(&save).map_err(|_| Error::Reason(Reason::NotInSpec))?;
        if blake2s_tagged(&bytes) == mark.state_hash {
            return Ok(());
        }
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if let Ok(now) = parse_save(text) {
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
    let report = replay_journal(path, None, Some(mark.state_rev))?;
    if blake2s_tagged(&report.now_yaml) != mark.state_hash {
        return Err(
            checkpoint_diverged(mark.state_rev, mark.round_index, "state hash differed")
                .replayed(report.now_yaml)
                .into(),
        );
    }
    let _write = lock_write(path, lock).map_err(Error::Reason)?;
    write_atomic(&save, &report.now_yaml).map_err(Error::Reason)?;
    Ok(())
}

impl From<crate::journal::Error> for Error {
    fn from(fault: crate::journal::Error) -> Self {
        if fault.reason == Reason::RulesRevMismatch {
            return Error::Reason(Reason::RulesRevMismatch);
        }
        Divergence::new(
            DivergenceKind::Journal,
            0,
            None,
            format!("{}: {}", fault.file, fault.detail),
        )
        .into()
    }
}

fn check_sequence(
    records: &[Record],
    cap: u64,
    base_rev: u64,
    engine_base: bool,
    base_bytes: &[u8],
) -> Result<(), Error> {
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
    let missing = |expected| {
        Divergence::new(DivergenceKind::Missing, expected, None, "no journal record")
            .replayed(base_bytes.to_vec())
    };
    let mut expected = 1u64;
    for (rev, before, index) in marks {
        if rev != expected {
            return Err(missing(expected).into());
        }
        let entry = &records[index];
        if before.saturating_add(1) != rev {
            return Err(Divergence::new(
                entry.into(),
                rev,
                entry.round_index(),
                "state_rev differed",
            )
            .replayed(base_bytes.to_vec())
            .into());
        }
        if engine_base && rev == base_rev {
            check_base_migrate(entry, base_bytes)?;
        }
        expected += 1;
    }
    if cap > 0 && expected.saturating_sub(1) != cap {
        return Err(missing(expected).into());
    }
    Ok(())
}

fn check_base_migrate(entry: &Record, base_bytes: &[u8]) -> Result<(), Error> {
    let Record::Migrate(record) = entry else {
        return Err(Divergence::new(
            entry.into(),
            entry.state_rev_after(),
            entry.round_index(),
            "base mark is not a migrate",
        )
        .replayed(base_bytes.to_vec())
        .into());
    };
    let migrate_diverged = |detail| {
        Divergence::new(
            DivergenceKind::Migrate,
            record.state_rev_after,
            None,
            detail,
        )
        .replayed(base_bytes.to_vec())
    };
    if record.now_hash != blake2s_tagged(base_bytes) {
        return Err(migrate_diverged("now_hash differed").into());
    }
    let base_now = load_save_bytes(base_bytes)?;
    if record.pack_rev_after != base_now.pack_rev {
        return Err(migrate_diverged("pack_rev differed").into());
    }
    Ok(())
}

pub(crate) fn replay_record(world: &mut World<()>, entry: &Record) -> Result<(), Error> {
    match entry {
        Record::Round(record) => apply_recorded_round(world, record),
        Record::Migrate(record) => apply_migrate(world, record),
        Record::Checkpoint(record) => {
            let written = emit_save_bytes(world)?;
            if blake2s_tagged(&written) != record.state_hash {
                return Err(checkpoint_diverged(
                    record.state_rev,
                    record.round_index,
                    "state hash differed",
                )
                .replayed(written)
                .into());
            }
            Ok(())
        }
    }
}

fn apply_recorded_round(world: &mut World<()>, record: &RoundRecord) -> Result<(), Error> {
    require_rules_rev(record.rules_rev).map_err(Error::Reason)?;
    if world.state().round_index + 1 != record.round_index {
        return Err(round_diverged(record, "round_index differed").into());
    }
    let seed = round_seed(&world.state().shard_id, record.round_index);
    if record.seed != seed {
        return Err(round_diverged(record, "seed differed").into());
    }
    let replayed = replay_round(
        world,
        &record.last_round_at,
        &record.intents,
        &Presence {
            humans: record.presence_humans,
            actors_present: record.actors_present.clone(),
        },
    )
    .map(|output| output.effects)
    .map_err(|code| Divergence {
        replayed: emit_save_bytes(world).ok(),
        ..round_diverged(record, code.as_str())
    })?;
    let effect_diff = first_effect_diff(&record.effects, &replayed);
    if world.state().state_rev != record.state_rev_after {
        return Err(Divergence {
            replayed: emit_save_bytes(world).ok(),
            ..round_diverged(record, "state_rev differed")
        }
        .into());
    }
    let written = emit_save_bytes(world)?;
    if blake2s_tagged(&written) != record.now_hash {
        let detail = effect_diff.unwrap_or_else(|| "now_hash differed".to_string());
        return Err(round_diverged(record, detail).replayed(written).into());
    }
    if let Some(detail) = effect_diff {
        return Err(round_diverged(record, detail).into());
    }
    Ok(())
}

pub(crate) fn first_effect_diff(logged: &[Effect], replayed: &[Effect]) -> Option<String> {
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

fn apply_migrate(world: &mut World<()>, record: &MigrateRecord) -> Result<(), Error> {
    let diverged = |detail| {
        Divergence::new(
            DivergenceKind::Migrate,
            record.state_rev_after,
            None,
            detail,
        )
    };
    if world.state().pack_rev != record.pack_rev_before {
        return Err(diverged("pack_rev differed").into());
    }
    if record.edited {
        // A migrate of a hand edit restarts from the checkpoint written with it.
        let path = journal::checkpoint_path(world.root(), record.state_rev_after);
        let bytes = fs::read(path).map_err(|_| diverged("missing snapshot"))?;
        if blake2s_tagged(&bytes) != record.now_hash {
            return Err(diverged("now_hash differed").replayed(bytes).into());
        }
        world.set_state(load_save_bytes(&bytes)?);
        return Ok(());
    }
    let next = migrate_save(
        world.state().clone(),
        &record.pack_rev_after,
        record.state_rev_after,
        &record.meters,
    );
    let bytes = emit_save(&next).map_err(Error::Reason)?.into_bytes();
    if blake2s_tagged(&bytes) != record.now_hash {
        return Err(diverged("now_hash differed").replayed(bytes).into());
    }
    world.set_state(next);
    Ok(())
}

pub(crate) fn load_save_bytes(bytes: &[u8]) -> Result<State, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Reason(Reason::Schema))?;
    parse_save(text).map_err(|diag| Error::Reason(first_reason(&diag)))
}

pub(crate) fn state_rev_of(bytes: &[u8]) -> Result<u64, Error> {
    Ok(load_save_bytes(bytes)?.state_rev)
}

pub(crate) fn emit_save_bytes<L>(world: &World<L>) -> Result<Vec<u8>, Error> {
    emit_save(world.state())
        .map(|text| text.into_bytes())
        .map_err(Error::Reason)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::replay_journal;
    use crate::round::play_pack_once;
    use crate::testing::copy_docks;

    #[test]
    fn two_replays_in_one_process_do_not_collide() {
        let pack = copy_docks("replay-collide");
        play_pack_once(&pack).unwrap();
        let left = replay_journal(&pack, None, None).unwrap();
        let right = replay_journal(&pack, None, None).unwrap();
        assert_eq!(left.now_yaml, right.now_yaml);
        assert_eq!(left.applied, 1);
        assert_eq!(right.applied, 1);
        assert_eq!(left.state_rev, 1);
        let _ = fs::remove_dir_all(&pack);
    }
}
