//! Walk the journal and rebuild the state it records.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::canon::load_canon;
use crate::diagnostic::{first_reason, Diagnostic};
use crate::ecs::{self, Presence, Save};
use crate::hash::blake2s_tagged;
use crate::rules::effect::Effect;
use crate::session;
use crate::store::journal;
use crate::store::records::{emit_effect, MigrateRecord, Record, RoundRecord};
use crate::store::require_rules_rev;
use crate::store::save;
use crate::Reason;

use crate::round::play::replay_round;

#[derive(Debug)]
#[cfg_attr(not(feature = "testing"), expect(unreachable_pub))]
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
    pub replayed_bytes: Option<Vec<u8>>,
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
            replayed_bytes: None,
            save_line: None,
        }
    }

    pub(crate) fn with_replayed_bytes(self, bytes: Vec<u8>) -> Self {
        Self {
            replayed_bytes: Some(bytes),
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

impl From<Reason> for Error {
    fn from(reason: Reason) -> Self {
        Self::Reason(reason)
    }
}

impl From<Vec<Diagnostic>> for Error {
    fn from(diagnostics: Vec<Diagnostic>) -> Self {
        Self::Reason(first_reason(&diagnostics))
    }
}

impl From<Error> for Reason {
    fn from(error: Error) -> Self {
        match error {
            Error::Reason(reason) => reason,
            Error::Diverged(_) => Reason::JournalDiverged,
        }
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

/// Replay the journal's `records`. `from` is a directory whose `state/save.yaml` replaces
/// the base. `until` ignores records with a later `state_rev`, so a live server cannot race
/// a verify.
#[cfg_attr(not(feature = "testing"), expect(unreachable_pub))]
pub fn apply_journal(
    pack: &Path,
    records: &[Record],
    from: Option<&Path>,
    until: Option<u64>,
) -> Result<Report, Error> {
    if !pack.join("pack.yaml").is_file() {
        return Err(Error::Reason(Reason::NotInSpec));
    }
    let base_path = journal::base_path(pack);
    let base_bytes = if let Some(from) = from {
        journal::read_save(&journal::save_path(from))?
    } else if base_path.exists() {
        journal::read_save(&base_path)?
    } else {
        let now_yaml = journal::read_save(&journal::save_path(pack))?;
        if records.is_empty() {
            return Ok(Report {
                state_rev: state_rev_of(&now_yaml)?,
                applied: 0,
                now_yaml,
            });
        }
        now_yaml
    };
    let base_rev = state_rev_of(&base_bytes)?;
    let cap = until.unwrap_or_else(|| {
        records
            .iter()
            .map(Record::state_rev_after)
            .max()
            .unwrap_or(base_rev)
    });
    check_sequence(records, cap, base_rev, from.is_none(), &base_bytes)?;
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
    let mut session = open_at(pack, &base_bytes)?;
    for entry in records.iter() {
        let rev = entry.state_rev_after();
        if rev <= base_rev || rev > cap {
            continue;
        }
        apply_record(&mut session, entry)?;
        if entry.changes_state() {
            applied += 1;
        }
    }
    Ok(Report {
        state_rev: session.world().state_rev,
        applied,
        now_yaml: emit_save_bytes(&session)?,
    })
}

/// A replay world at `base` with the pack's canon.
pub(crate) fn open_at(pack: &Path, base: &[u8]) -> Result<session::Session<()>, Error> {
    let save = load_save_bytes(base)?;
    let canon = load_canon(pack)?;
    Ok(session::Session::from_parts(
        pack,
        ecs::World::new(save, Arc::new(canon)),
    ))
}

impl From<crate::store::journal::Error> for Error {
    fn from(fault: crate::store::journal::Error) -> Self {
        if fault.reason != Reason::JournalDiverged {
            return Error::Reason(fault.reason);
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
            .with_replayed_bytes(base_bytes.to_vec())
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
            .with_replayed_bytes(base_bytes.to_vec())
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
        .with_replayed_bytes(base_bytes.to_vec())
        .into());
    };
    let migrate_diverged = |detail| {
        Divergence::new(
            DivergenceKind::Migrate,
            record.state_rev_after,
            None,
            detail,
        )
        .with_replayed_bytes(base_bytes.to_vec())
    };
    if record.state_hash != blake2s_tagged(base_bytes) {
        return Err(migrate_diverged("state_hash differed").into());
    }
    let base_state = load_save_bytes(base_bytes)?;
    if record.pack_rev_after != base_state.pack_rev {
        return Err(migrate_diverged("pack_rev differed").into());
    }
    Ok(())
}

pub(crate) fn apply_record(
    session: &mut session::Session<()>,
    entry: &Record,
) -> Result<(), Error> {
    match entry {
        Record::Round(record) => apply_recorded_round(session, record),
        Record::Migrate(record) => apply_migrate(session, record),
        Record::Checkpoint(record) => {
            let written = emit_save_bytes(session)?;
            if blake2s_tagged(&written) != record.state_hash {
                return Err(checkpoint_diverged(
                    record.state_rev,
                    record.round_index,
                    "state hash differed",
                )
                .with_replayed_bytes(written)
                .into());
            }
            Ok(())
        }
    }
}

fn apply_recorded_round(
    session: &mut session::Session<()>,
    record: &RoundRecord,
) -> Result<(), Error> {
    require_rules_rev(record.rules_rev)?;
    if session.world().round_index + 1 != record.round_index {
        return Err(round_diverged(record, "round_index differed").into());
    }
    let replayed = replay_round(
        session,
        &record.last_round_at,
        &record.intents,
        &Presence {
            players: record.presence_players,
            actors_present: record.actors_present.clone(),
        },
    )
    .map(|output| output.effects)
    .map_err(|code| Divergence {
        replayed_bytes: emit_save_bytes(session).ok(),
        ..round_diverged(record, code.as_str())
    })?;
    let effect_diff = first_effect_diff(&record.effects, &replayed);
    if session.world().state_rev != record.state_rev_after {
        return Err(Divergence {
            replayed_bytes: emit_save_bytes(session).ok(),
            ..round_diverged(record, "state_rev differed")
        }
        .into());
    }
    let written = emit_save_bytes(session)?;
    if blake2s_tagged(&written) != record.state_hash {
        let detail = effect_diff.unwrap_or_else(|| "state_hash differed".to_string());
        return Err(round_diverged(record, detail)
            .with_replayed_bytes(written)
            .into());
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

fn apply_migrate(session: &mut session::Session<()>, record: &MigrateRecord) -> Result<(), Error> {
    let diverged = |detail| {
        Divergence::new(
            DivergenceKind::Migrate,
            record.state_rev_after,
            None,
            detail,
        )
    };
    if session.world().pack_rev != record.pack_rev_before {
        return Err(diverged("pack_rev differed").into());
    }
    if record.edited {
        let path = journal::checkpoint_path(session.root(), record.state_rev_after);
        let bytes = fs::read(path).map_err(|_| diverged("missing snapshot"))?;
        if blake2s_tagged(&bytes) != record.state_hash {
            return Err(diverged("state_hash differed")
                .with_replayed_bytes(bytes)
                .into());
        }
        let canon = Arc::clone(session.world().canon());
        session.replace_world(ecs::World::new(load_save_bytes(&bytes)?, canon));
        return Ok(());
    }
    let next = save::migrate(session.world().clone(), record)?;
    let bytes = save::emit(&next)?.into_bytes();
    if blake2s_tagged(&bytes) != record.state_hash {
        return Err(diverged("state_hash differed")
            .with_replayed_bytes(bytes)
            .into());
    }
    session.replace_world(next);
    Ok(())
}

pub(crate) fn load_save_bytes(bytes: &[u8]) -> Result<Save, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Reason::Schema)?;
    Ok(save::parse(text)?)
}

pub(crate) fn state_rev_of(bytes: &[u8]) -> Result<u64, Error> {
    Ok(load_save_bytes(bytes)?.state_rev)
}

pub(crate) fn emit_save_bytes<Lock>(session: &session::Session<Lock>) -> Result<Vec<u8>, Error> {
    Ok(save::emit(session.world())?.into_bytes())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::apply_journal;
    use crate::round::play_pack_once;
    use crate::store::journal;
    use crate::testing::copy_pack;

    #[test]
    fn two_replays_in_one_process_do_not_collide() {
        let pack = copy_pack("docks", "replay-collide");
        play_pack_once(&pack).unwrap();
        let records = journal::read_records(&pack).unwrap();
        let left = apply_journal(&pack, &records, None, None).unwrap();
        let right = apply_journal(&pack, &records, None, None).unwrap();
        assert_eq!(left.now_yaml, right.now_yaml);
        assert_eq!(left.applied, 1);
        assert_eq!(right.applied, 1);
        assert_eq!(left.state_rev, 1);
        let _ = fs::remove_dir_all(&pack);
    }
}
