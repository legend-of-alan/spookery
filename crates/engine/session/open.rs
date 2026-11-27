//! Turn a pack directory into a session at the journal tip, and migrate it.

use std::path::Path;
use std::sync::Arc;

use super::Session;
use crate::canon;
use crate::ecs;
use crate::hash::blake2s_tagged;
use crate::replay::{
    apply_journal, apply_record, checkpoint_diverged, load_save_bytes, open_at, Error,
};
use crate::store::durability::write_atomic;
use crate::store::journal;
use crate::store::lock::{self, PackGuard};
use crate::store::records::Record;
use crate::store::save;
use crate::Reason;

/// Open the pack at the journal tip. `save.yaml` stays the last checkpoint.
/// Records after that checkpoint are in the live bundle, so closed bundles stay unread.
/// The caller holds the pack lock, so a crash is repaired and the checkpoint committed.
pub(crate) fn load_tip(pack: &Path, guard: &PackGuard) -> Result<Session<()>, Reason> {
    choose_seed(pack, guard)?;
    rebuild_save(pack, guard)?;
    let session = peek_tip(pack)?;
    crate::store::commit_last_checkpoint(&session, &lock::take_write(session.root(), guard)?)?;
    Ok(session)
}

/// Write a fresh dice seed into `save.yaml` when it has none and the journal is still empty,
/// so the first round, and replay from the base save, roll from it. A seed already there stays.
fn choose_seed(pack: &Path, guard: &PackGuard) -> Result<(), Error> {
    if journal::read_mark(pack)?.is_some() || journal::find_live_bundle(pack).is_some() {
        return Ok(());
    }
    let save_path = journal::save_path(pack);
    let mut file_save = load_save_bytes(&journal::read_save(&save_path)?)?;
    if !file_save.seed.is_empty() {
        return Ok(());
    }
    file_save.seed = uuid::Uuid::new_v4().simple().to_string();
    let _write = lock::take_write(pack, guard)?;
    let text = save::emit(&file_save)?;
    Ok(write_atomic(&save_path, text.as_bytes())?)
}

/// The journal tip in memory. Writes nothing.
pub(crate) fn peek_tip(pack: &Path) -> Result<Session<()>, Error> {
    let mut session = Session::open(pack)?;
    if apply_live(&mut session)? {
        save::check(session.world())?;
    }
    Ok(session)
}

/// Migrate the journal tip. A save that the journal has not passed is migrated as it stands.
pub fn migrate_pack(pack: &Path) -> Result<(), Reason> {
    let guard = lock::take(pack)?;
    choose_seed(pack, &guard)?;
    let (world, edited) = load_migrate_tip(pack, &guard)?;
    crate::store::commit_migrate(pack, world, edited, &guard)?;
    Ok(())
}

/// The world to migrate, and whether it is a hand edit of the checkpoint mark. A hand edit
/// with live rounds past it is refused, because the tip does not hold it.
fn load_migrate_tip(pack: &Path, guard: &PackGuard) -> Result<(ecs::World, bool), Error> {
    let bytes = journal::read_save(&journal::save_path(pack))?;
    let file_save = load_save_bytes(&bytes)?;
    let edited = journal::edits_mark(pack, file_save.state_rev, &bytes)?;
    let ahead = journal::read_live_records(pack)?.iter().any(|record| {
        !matches!(record, Record::Checkpoint(_)) && record.state_rev_after() > file_save.state_rev
    });
    match (ahead, edited) {
        (true, true) => Err(Reason::HandEdit.into()),
        (true, false) => Ok((load_tip_unchecked(pack, guard)?.world().clone(), false)),
        (false, _) => Ok((
            ecs::World::new(file_save, Arc::new(canon::load(pack)?)),
            edited,
        )),
    }
}

/// The journal tip in memory and unchecked. Migrate checks it after adding its meters.
fn load_tip_unchecked(pack: &Path, guard: &PackGuard) -> Result<Session<()>, Error> {
    rebuild_save(pack, guard)?;
    let bytes = journal::read_save(&journal::save_path(pack))?;
    let mut session = open_at(pack, &bytes)?;
    apply_live(&mut session)?;
    crate::store::commit_last_checkpoint(&session, &lock::take_write(session.root(), guard)?)?;
    Ok(session)
}

/// Replay the live records past the save. `true` when any record was replayed.
fn apply_live(session: &mut Session<()>) -> Result<bool, Error> {
    let records = journal::read_live_records(session.root())?;
    let base_rev = session.world().state_rev;
    let mut replayed = false;
    for record in &records {
        if record.state_rev_after() <= base_rev {
            continue;
        }
        apply_record(session, record)?;
        replayed = true;
    }
    Ok(replayed)
}

fn rebuild_save(pack: &Path, guard: &PackGuard) -> Result<(), Error> {
    let Some(mark) = journal::read_mark(pack)? else {
        return Ok(());
    };
    let save_path = journal::save_path(pack);
    match journal::read_save(&save_path) {
        Ok(bytes) => {
            if blake2s_tagged(&bytes) == mark.state_hash {
                return Ok(());
            }
            if let Ok(text) = std::str::from_utf8(&bytes) {
                if let Ok(file_save) = save::parse(text) {
                    if file_save.state_rev == mark.state_rev
                        || save::emit(&file_save)
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
        Err(Reason::NotInSpec) => {}
        Err(reason) => return Err(reason.into()),
    }
    let records = journal::read_records(pack)?;
    let report = apply_journal(pack, &records, None, Some(mark.state_rev))?;
    if blake2s_tagged(&report.now_yaml) != mark.state_hash {
        return Err(
            checkpoint_diverged(mark.state_rev, mark.round_index, "state hash differed")
                .with_replayed_bytes(report.now_yaml)
                .into(),
        );
    }
    let _write = lock::take_write(pack, guard)?;
    Ok(write_atomic(&save_path, &report.now_yaml)?)
}
