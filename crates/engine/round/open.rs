//! Turn a pack directory into a session at the journal tip, and migrate it.

use std::path::Path;
use std::sync::Arc;

use crate::canon::load_canon;
use crate::ecs;
use crate::hash::blake2s_tagged;
use crate::round::replay::{
    apply_journal, apply_record, checkpoint_diverged, load_save_bytes, open_at, Error,
};
use crate::session;
use crate::store::durability::write_atomic;
use crate::store::journal;
use crate::store::lock::{self, lock_pack, PackGuard};
use crate::store::records::Record;
use crate::store::save;
use crate::Reason;

/// Open the pack at the journal tip. `save.yaml` stays the last checkpoint.
/// Records after that checkpoint are in the live bundle, so closed bundles stay unread.
/// The caller holds the pack lock, so a crash is repaired and the checkpoint settled.
pub(crate) fn load_tip(pack: &Path, guard: &PackGuard) -> Result<session::Session<()>, Reason> {
    choose_seed(pack, guard)?;
    rebuild_save(pack, guard)?;
    let session = peek_at_tip(pack)?;
    crate::store::commit::settle_checkpoint(&session, &lock::take_write(session.root(), guard)?)?;
    Ok(session)
}

/// Write a fresh dice seed into `save.yaml` when it has none and the journal is still empty,
/// so the first round, and replay from the base save, roll from it. A seed already there stays.
fn choose_seed(pack: &Path, guard: &PackGuard) -> Result<(), Error> {
    if journal::read_head(pack)?.is_some() || journal::find_live_bundle(pack).is_some() {
        return Ok(());
    }
    let save_path = journal::save_path(pack);
    let mut save = load_save_bytes(&journal::read_save(&save_path)?)?;
    if !save.seed.is_empty() {
        return Ok(());
    }
    save.seed = uuid::Uuid::new_v4().simple().to_string();
    let _write = lock::take_write(pack, guard)?;
    Ok(write_atomic(&save_path, save::emit(&save)?.as_bytes())?)
}

/// The journal tip in memory. Writes nothing.
pub(crate) fn peek_at_tip(pack: &Path) -> Result<session::Session<()>, Error> {
    let mut session = session::Session::open(pack)?;
    if apply_tail(&mut session)? {
        save::check(session.world())?;
    }
    Ok(session)
}

/// Migrate the journal tip. A save that the journal has not passed is migrated as it stands.
pub fn migrate_pack(pack: &Path) -> Result<(), Reason> {
    let guard = lock_pack(pack)?;
    choose_seed(pack, &guard)?;
    let (world, edited) = load_migrate_tip(pack, &guard)?;
    crate::store::commit::seal_migrate(pack, world, edited, &guard)?;
    Ok(())
}

/// The world to migrate, and whether it is a hand edit of the checkpoint head. A hand edit
/// with live rounds past it is refused, because the tip does not hold it.
fn load_migrate_tip(pack: &Path, guard: &PackGuard) -> Result<(ecs::World, bool), Error> {
    let bytes = journal::read_save(&journal::save_path(pack))?;
    let file_save = load_save_bytes(&bytes)?;
    let edited = journal::edits_head(pack, file_save.state_rev, &bytes)?;
    let ahead = journal::read_live_records(pack)?.iter().any(|record| {
        !matches!(record, Record::Checkpoint(_)) && record.state_rev_after() > file_save.state_rev
    });
    match (ahead, edited) {
        (true, true) => Err(Reason::HandEdit.into()),
        (true, false) => Ok((load_tip_unchecked(pack, guard)?.world().clone(), false)),
        (false, _) => Ok((
            ecs::World::new(file_save, Arc::new(load_canon(pack)?)),
            edited,
        )),
    }
}

/// The journal tip in memory and unchecked. Migrate checks it after adding its meters.
fn load_tip_unchecked(pack: &Path, guard: &PackGuard) -> Result<session::Session<()>, Error> {
    rebuild_save(pack, guard)?;
    let save = journal::read_save(&journal::save_path(pack))?;
    let mut session = open_at(pack, &save)?;
    apply_tail(&mut session)?;
    crate::store::commit::settle_checkpoint(&session, &lock::take_write(session.root(), guard)?)?;
    Ok(session)
}

/// Replay the live records past the save. `true` when any record was replayed.
fn apply_tail(session: &mut session::Session<()>) -> Result<bool, Error> {
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
    let Some(head) = journal::read_head(pack)? else {
        return Ok(());
    };
    let save_path = journal::save_path(pack);
    match journal::read_save(&save_path) {
        Ok(bytes) => {
            if blake2s_tagged(&bytes) == head.state_hash {
                return Ok(());
            }
            if let Ok(text) = std::str::from_utf8(&bytes) {
                if let Ok(save) = save::parse(text) {
                    if save.state_rev == head.state_rev
                        || save::emit(&save)
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
    let report = apply_journal(pack, &records, None, Some(head.state_rev))?;
    if blake2s_tagged(&report.now_yaml) != head.state_hash {
        return Err(
            checkpoint_diverged(head.state_rev, head.round_index, "state hash differed")
                .with_replayed_bytes(report.now_yaml)
                .into(),
        );
    }
    let _write = lock::take_write(pack, guard)?;
    Ok(write_atomic(&save_path, &report.now_yaml)?)
}
