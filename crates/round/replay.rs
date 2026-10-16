use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use spookery_kernel::canon::{bind_inventory, bind_meters, pack_rev};
use spookery_kernel::commit::replay_write_now;
use spookery_kernel::diagnostic::reason_of;
use spookery_kernel::effect::Effect;
use spookery_kernel::journal::{
    self, hash_now, JournalEntry, MigrateRecord, OldPresence, TickRecord,
};
use spookery_kernel::model::{Now, Presence};
use spookery_kernel::store::codec::{
    canonicalize, emit_effect, emit_save, load_save, with_presence, without_presence,
};
use spookery_kernel::store::require_rules_rev;
use spookery_kernel::world::World;
use spookery_kernel::Reason;

use crate::play::{play_recorded, RecordedRound};
use crate::round_seed::round_seed;

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
    if !pack.join("pack.yaml").is_file() || !pack.join("state/save.yaml").is_file() {
        return Err(ReplayError::Reason(Reason::NotInSpec));
    }
    let records = load_entries(pack)?;
    let old_presence = load_old_presence(pack)?;
    let base_path = pack.join("state").join(journal::base_rel());
    let base_bytes = if let Some(from) = from {
        read_from_now(from)?
    } else if base_path.is_file() {
        fs::read(&base_path).map_err(|_| ReplayError::Reason(Reason::Schema))?
    } else if records.is_empty() && old_presence.is_empty() {
        let now_yaml = fs::read(pack.join("state/save.yaml"))
            .map_err(|_| ReplayError::Reason(Reason::NotInSpec))?;
        return Ok(ReplayReport {
            state_rev: state_rev_of(&now_yaml)?,
            applied: 0,
            now_yaml,
        });
    } else {
        return Err(ReplayError::Diverged(diverge(
            0,
            "base",
            None,
            "missing base.yaml".to_string(),
            None,
        )));
    };
    let base_rev = state_rev_of(&base_bytes)?;
    let cap = until.unwrap_or_else(|| {
        records
            .iter()
            .map(JournalEntry::state_rev_after)
            .chain(old_presence.iter().map(|presence| presence.state_rev_after))
            .filter(|rev| until_allows(*rev, until))
            .max()
            .unwrap_or(base_rev)
    });
    check_sequence(
        &records,
        &old_presence,
        cap,
        base_rev,
        from.is_none(),
        &base_bytes,
    )?;
    let mut applied = 0u64;
    let mut saved = Vec::new();
    let needs_apply = records.iter().any(|entry| {
        let rev = entry.state_rev_after();
        rev > base_rev && rev <= cap
    }) || old_presence
        .iter()
        .any(|presence| presence.state_rev_after > base_rev && presence.state_rev_after <= cap);
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
    // Both lists are sorted by `state_rev_after`, so walk them together.
    let mut entries = records
        .iter()
        .skip_while(|entry| entry.state_rev_after() <= base_rev)
        .peekable();
    let mut old = old_presence
        .iter()
        .skip_while(|presence| presence.state_rev_after <= base_rev)
        .peekable();
    for rev in (base_rev + 1)..=cap {
        if let Some(presence) = old.next_if(|presence| presence.state_rev_after == rev) {
            apply_old_presence(&mut world, presence)?;
            saved = fs::read(temp.0.join("state/save.yaml"))
                .map_err(|_| ReplayError::Reason(Reason::Schema))?;
            continue;
        }
        let entry = entries
            .next_if(|entry| entry.state_rev_after() == rev)
            .ok_or_else(|| {
                ReplayError::Diverged(diverge(
                    rev,
                    "missing",
                    None,
                    "no journal record".to_string(),
                    None,
                ))
            })?;
        apply(&mut world, entry)?;
        applied += 1;
        let now_yaml = fs::read(temp.0.join("state/save.yaml"))
            .map_err(|_| ReplayError::Reason(Reason::Schema))?;
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

/// Read `save.yaml` once, replay up to that `state_rev`, and byte-compare.
pub fn verify_pack(pack: &Path, from: Option<&Path>) -> Result<VerifyReport, VerifyError> {
    if !pack.join("pack.yaml").is_file() || !pack.join("state/save.yaml").is_file() {
        return Err(VerifyError::Replay(ReplayError::Reason(Reason::NotInSpec)));
    }
    let live = fs::read(pack.join("state/save.yaml"))
        .map_err(|_| VerifyError::Replay(ReplayError::Reason(Reason::NotInSpec)))?;
    let until = state_rev_of(&live).map_err(VerifyError::Replay)?;
    if from.is_none() && !pack.join("state").join(journal::base_rel()).is_file() {
        let records = load_entries(pack).map_err(VerifyError::Replay)?;
        let old_presence = load_old_presence(pack).map_err(VerifyError::Replay)?;
        if records.is_empty() && old_presence.is_empty() {
            return Ok(VerifyReport {
                state_rev: until,
                applied: 0,
            });
        }
    }
    let report = match from_journal(pack, from, Some(until)) {
        Ok(report) => report,
        Err(ReplayError::Diverged(diverged)) => return Err(with_diff(diverged, &live)),
        Err(err) => return Err(VerifyError::Replay(err)),
    };
    if !saves_match(&report.now_yaml, &live) {
        let diverged = diverge(
            report.state_rev,
            "save",
            None,
            "save.yaml differed".to_string(),
            Some(report.now_yaml.clone()),
        );
        return Err(with_diff(diverged, &live));
    }
    Ok(VerifyReport {
        state_rev: report.state_rev,
        applied: report.applied,
    })
}

fn until_allows(rev: u64, until: Option<u64>) -> bool {
    until.is_none_or(|cap| rev <= cap)
}

fn load_entries(pack: &Path) -> Result<Vec<JournalEntry>, ReplayError> {
    journal::load_journal(pack).map_err(journal_fault)
}

fn load_old_presence(pack: &Path) -> Result<Vec<OldPresence>, ReplayError> {
    journal::load_old_presence(pack).map_err(journal_fault)
}

fn journal_fault(fault: spookery_kernel::journal::JournalFault) -> ReplayError {
    ReplayError::Diverged(diverge(
        0,
        "journal",
        None,
        format!("{}: {}", fault.file, fault.detail),
        None,
    ))
}

/// `bytes` come from `emit_save`, which no longer writes presence. New records
/// hash those bytes. Old records hashed them with the presence block put back.
fn save_hash_matches(bytes: &[u8], expected: &str, presence: &Presence) -> bool {
    if hash_now(bytes) == expected {
        return true;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let legacy = with_presence(text, presence.humans, &presence.actors_present);
    hash_now(legacy.as_bytes()) == expected
}

fn saves_match(replayed: &[u8], live: &[u8]) -> bool {
    match (std::str::from_utf8(replayed), std::str::from_utf8(live)) {
        (Ok(left), Ok(right)) => without_presence(left) == without_presence(right),
        _ => replayed == live,
    }
}

fn check_sequence(
    records: &[JournalEntry],
    old_presence: &[OldPresence],
    cap: u64,
    base_rev: u64,
    engine_base: bool,
    base_bytes: &[u8],
) -> Result<(), ReplayError> {
    let mut marks: Vec<(u64, u64, Option<usize>)> = Vec::new();
    for presence in old_presence {
        if presence.state_rev_after > cap {
            continue;
        }
        marks.push((presence.state_rev_after, presence.state_rev_before, None));
    }
    for (index, entry) in records.iter().enumerate() {
        let rev = entry.state_rev_after();
        if rev > cap {
            continue;
        }
        marks.push((rev, entry.state_rev_before(), Some(index)));
    }
    marks.sort_by_key(|mark| mark.0);
    let mut expected = 1u64;
    for (rev, before, entry) in marks {
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
            let kind = entry
                .map(|index| records[index].kind_name())
                .unwrap_or("presence");
            let tick = entry.and_then(|index| records[index].tick_index());
            return Err(ReplayError::Diverged(diverge(
                rev,
                kind,
                tick,
                "state_rev differed".to_string(),
                Some(base_bytes.to_vec()),
            )));
        }
        if engine_base && rev == base_rev {
            if let Some(index) = entry {
                check_mark(&records[index], base_bytes)?;
            }
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
    if !save_hash_matches(&written, &record.now_hash, world.presence()) {
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

/// Restore who was seated from an old presence record and check its `now_hash`.
fn apply_old_presence(world: &mut World, old: &OldPresence) -> Result<(), ReplayError> {
    world.set_presence(Presence {
        humans: old.humans,
        actors_present: old.actors_present.clone(),
    });
    let mut next = world.now().clone();
    next.state_rev = old.state_rev_after;
    canonicalize(&mut next);
    let bytes = emit_save(&next).map_err(ReplayError::Reason)?.into_bytes();
    if !save_hash_matches(&bytes, &old.now_hash, world.presence()) {
        return Err(ReplayError::Diverged(diverge(
            old.state_rev_after,
            "presence",
            None,
            "now_hash differed".to_string(),
            Some(bytes),
        )));
    }
    replay_write_now(world, next).map_err(ReplayError::Reason)
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
    if !save_hash_matches(&bytes, &record.now_hash, world.presence()) {
        return Err(ReplayError::Diverged(diverge(
            record.state_rev_after,
            "migrate",
            None,
            "now_hash differed".to_string(),
            Some(bytes),
        )));
    }
    replay_write_now(world, next).map_err(ReplayError::Reason)
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
    fs::read(world.root().join("state/save.yaml")).map_err(|_| ReplayError::Reason(Reason::Schema))
}

fn read_now(world: &World) -> Option<Vec<u8>> {
    fs::read(world.root().join("state/save.yaml")).ok()
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
        let path = pack.join("state/journal/tick-0001.jsonl");
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
