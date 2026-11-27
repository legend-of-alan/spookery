//! Publish a round or a migrate: the journal record, then at a checkpoint the save.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::canon;
use crate::diagnostic;
use crate::ecs::{self, sort_unique};
use crate::hash::blake2s_tagged;
use crate::session;
use crate::store::durability::{fsync_dir, fsync_file, refuse_io, write_atomic};
use crate::store::journal;
use crate::store::lock::{self, PackGuard, WriteGuard};
use crate::store::records::{MigrateRecord, Record, RoundRecord};
use crate::store::recovery::close_checkpointed;
use crate::store::save;
use crate::Reason;

pub struct Outcome {
    pub state_rev: u64,
    pub journal_path: String,
    /// The committed round's index and the world time it ended at.
    pub round_index: u64,
    pub world_time: u64,
}

/// Commit one played round, append it, end a due checkpoint, and clear the queue. `world` is
/// the round's working copy of the save, published only once the round is on disk.
pub(crate) fn commit_round(
    session: &mut session::Session,
    mut world: ecs::World,
    record: RoundRecord,
) -> Result<Outcome, Reason> {
    let started = std::time::Instant::now();
    let write = lock::take_write(session.root(), session.lock())?;
    publish_last_checkpoint(session, &write)?;
    bump_state_rev(&mut world)?;
    let now_bytes = save::emit(&world)?.into_bytes();
    let line = build_round_line(
        record,
        session.world().state_rev,
        &mut world,
        &now_bytes,
        session.root(),
    )?;
    let due = is_checkpoint_due(session.canon().checkpoint_every, world.round_index);
    let (payload, chain) = if due {
        let (payload, chain) = commit_checkpoint(line, &world, &now_bytes)?;
        (payload, Some(chain))
    } else {
        (line, None)
    };
    let mut journal_path = append_to_bundle(session.root(), &payload, &write)?;
    session.replace_world(world);
    if let Some(chain) = chain {
        if warn_if_left_open(end_checkpoint(session, &now_bytes, &chain, &write))
            && !journal_path.ends_with(".gz")
        {
            journal_path.push_str(".gz");
        }
    }
    let world = session.world();
    tracing::info!(
        round_index = world.round_index,
        world_time = world.world_time,
        state_rev = world.state_rev,
        checkpoint_committed = due,
        elapsed_ms = started.elapsed().as_millis(),
        "round committed"
    );
    Ok(Outcome {
        state_rev: world.state_rev,
        journal_path,
        round_index: world.round_index,
        world_time: world.world_time,
    })
}

/// The journal already has this round. Keep the new state in memory.
pub(crate) fn keep_in_memory(
    session: &mut session::Session<()>,
    mut world: ecs::World,
) -> Result<(), Reason> {
    bump_state_rev(&mut world)?;
    session.replace_world(world);
    Ok(())
}

fn bump_state_rev(world: &mut ecs::World) -> Result<(), Reason> {
    world.state_rev = world.state_rev.checked_add(1).ok_or(Reason::OutOfRange)?;
    Ok(())
}

/// Commit a migrate record for `world`, which is already the journal tip or the on-disk save.
/// `edited` means `world` is a hand edit of the checkpoint mark. The journal does not hold
/// that edit, so a checkpoint of the migrated save follows the record and replay restarts
/// from it. Its snapshot is on disk before the record, so a crash after the append still
/// replays. The caller holds the pack lock.
pub(crate) fn commit_migrate(
    root: &Path,
    world: ecs::World,
    edited: bool,
    lock: &PackGuard,
) -> Result<(), Reason> {
    let pack_rev_after = canon::hash(root)?;
    let pack_rev_before = world.pack_rev.clone();
    let state_rev_before = world.state_rev;
    let state_rev_after = state_rev_before.checked_add(1).ok_or(Reason::OutOfRange)?;
    let stale = save::find_stale_modifiers(&world).into_iter();
    let mut dropped_modifiers: Vec<String> = stale.map(|(_, id, _)| id.to_string()).collect();
    sort_unique(&mut dropped_modifiers);
    let mut record = MigrateRecord {
        pack_rev_before,
        pack_rev_after,
        state_rev_before,
        state_rev_after,
        state_hash: String::new(),
        dropped_modifiers,
        dropped_meters: save::find_stale_meters(&world),
        edited,
        prev: String::new(),
        chain: String::new(),
    };
    let world = save::migrate(world, &record)?;
    save::check(&world).map_err(|diagnostics| diagnostic::find_reason(&diagnostics))?;
    let emitted = save::emit(&world)?.into_bytes();
    record.state_hash = blake2s_tagged(&emitted);
    let entry = Record::Migrate(record);
    let write = lock::take_write(root, lock)?;
    let prev = journal::read_chain_tip(root).map_err(|fault| fault.reason)?;
    let line = journal::commit(&entry, &prev);
    if edited {
        write_atomic(&journal::checkpoint_path(root, world.state_rev), &emitted)?;
        let (payload, chain) = commit_checkpoint(line, &world, &emitted)?;
        append_to_bundle(root, &payload, &write)?;
        store_checkpoint(
            root,
            &emitted,
            world.state_rev,
            world.round_index,
            &chain,
            &write,
        )?;
        warn_if_left_open(close_checkpointed(root, &write));
    } else {
        append_to_bundle(root, &line, &write)?;
    }
    tracing::info!(
        pack_rev = world.pack_rev.as_str(),
        state_rev = world.state_rev,
        operator_edit_trusted = edited,
        "pack migrated"
    );
    Ok(())
}

/// Follow a committed `line` with a checkpoint of `world`. Returns both lines and the
/// checkpoint's chain.
fn commit_checkpoint(
    line: String,
    world: &ecs::World,
    now_bytes: &[u8],
) -> Result<(String, String), Reason> {
    let prev = journal::read_chain(&line).map_err(|fault| fault.reason)?;
    let mark = journal::checkpoint_record(
        world.round_index,
        world.state_rev,
        &blake2s_tagged(now_bytes),
    );
    let committed = journal::commit(&mark, &prev);
    let chain = journal::read_chain(&committed).map_err(|fault| fault.reason)?;
    Ok((format!("{line}{committed}"), chain))
}

fn build_round_line(
    mut record: RoundRecord,
    state_rev_before: u64,
    world: &mut ecs::World,
    now_bytes: &[u8],
    root: &Path,
) -> Result<String, Reason> {
    record.state_rev_before = state_rev_before;
    record.state_rev_after = world.state_rev;
    record.memories = world.take_memories();
    record.state_hash = blake2s_tagged(now_bytes);
    let prev = journal::read_chain_tip(root).map_err(|fault| fault.reason)?;
    Ok(journal::commit(&Record::Round(record), &prev))
}

fn is_checkpoint_due(every: u64, round_index: u64) -> bool {
    every > 0 && round_index > 0 && round_index.is_multiple_of(every)
}

fn append_to_bundle(root: &Path, payload: &str, write: &WriteGuard) -> Result<String, Reason> {
    warn_if_left_open(close_checkpointed(root, write));
    let path = journal::next_bundle_path(root);
    let created = !path.is_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(refuse_io)?;
    }
    {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(refuse_io)?;
        file.write_all(payload.as_bytes()).map_err(refuse_io)?;
    }
    fsync_file(&path)?;
    if created {
        if let Some(parent) = path.parent() {
            fsync_dir(parent)?;
        }
    }
    Ok(path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/"))
}

fn store_checkpoint(
    root: &Path,
    now_bytes: &[u8],
    state_rev: u64,
    round_index: u64,
    chain: &str,
    _write: &WriteGuard,
) -> Result<(), Reason> {
    let mark = serde_json::to_string(&journal::Mark {
        rules_rev: crate::store::RULES_REV,
        state_rev,
        round_index,
        state_hash: blake2s_tagged(now_bytes),
        chain: chain.to_string(),
    })
    .map_err(|_| Reason::Schema)?;
    write_atomic(&journal::mark_path(root), mark.as_bytes())?;
    let save = journal::save_path(root);
    let base = journal::base_path(root);
    if !base.is_file() {
        fs::copy(&save, &base).map_err(refuse_io)?;
        fsync_file(&base)?;
        if let Some(parent) = base.parent() {
            fsync_dir(parent)?;
        }
    }
    let snap = journal::checkpoint_path(root, state_rev);
    if let Some(parent) = snap.parent() {
        fs::create_dir_all(parent).map_err(refuse_io)?;
    }
    write_atomic(&save, now_bytes)?;
    write_atomic(&snap, now_bytes)?;
    Ok(())
}

/// The journal line is already durable. An error means `save.yaml` or the gzip did
/// not land. The round stays committed either way.
fn end_checkpoint(
    session: &mut session::Session,
    now_bytes: &[u8],
    chain: &str,
    write: &WriteGuard,
) -> Result<(), Reason> {
    let (rev, round) = (session.world().state_rev, session.world().round_index);
    store_checkpoint(session.root(), now_bytes, rev, round, chain, write)?;
    close_checkpointed(session.root(), write)
}

/// A checkpointed bundle that could not be closed stays open, and the next commit closes it.
/// Returns whether it closed.
fn warn_if_left_open(closed: Result<(), Reason>) -> bool {
    if let Err(code) = &closed {
        tracing::warn!(code = %code, "a checkpointed journal bundle stays open until the next commit");
    }
    closed.is_ok()
}

/// Publish `save.yaml` and the snapshot for a checkpoint that is already in the
/// journal, then close that bundle. The next round can then start a new file.
pub(crate) fn publish_last_checkpoint<Lock>(
    session: &session::Session<Lock>,
    write: &WriteGuard,
) -> Result<(), Reason> {
    let Some(path) = journal::find_live_bundle(session.root()) else {
        return Ok(());
    };
    let Some(mark) = journal::trailing_checkpoint(&path) else {
        return Ok(());
    };
    let now_bytes = save::emit(session.world())?.into_bytes();
    if blake2s_tagged(&now_bytes) != mark.state_hash {
        return Ok(());
    }
    let save = journal::save_path(session.root());
    let snap = journal::checkpoint_path(session.root(), mark.state_rev);
    let save_ok = journal::hashes_to(&save, &mark.state_hash);
    let snap_ok = journal::hashes_to(&snap, &mark.state_hash);
    if !save_ok || !snap_ok {
        store_checkpoint(
            session.root(),
            &now_bytes,
            mark.state_rev,
            mark.round_index,
            &mark.chain,
            write,
        )?;
    }
    warn_if_left_open(close_checkpointed(session.root(), write));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::store::durability;
    use crate::store::records::RoundRecord;
    use crate::testing::copy_pack;

    use super::commit_round;

    #[test]
    fn a_failed_gzip_keeps_the_round_and_the_next_commit_starts_a_bundle() {
        let root = copy_pack("docks", "gzip-fail");
        let _clear = FailGuard;
        set_checkpoint_every(&root, 1);
        let mut session = crate::testing::resume(&root);
        let mut world = session.start_round();
        world.round_index = 1;
        durability::fail_next(".jsonl.gz.tmp");
        commit_round(&mut session, world, round_line(1)).unwrap();
        let live = crate::store::journal::read_live_records(&root).unwrap();
        let round_rev = live.iter().rev().find_map(|entry| match entry {
            crate::store::records::Record::Round(record) => Some(record.state_rev_after),
            _ => None,
        });
        let mark = crate::store::journal::latest_checkpoint(&live).unwrap();
        assert_eq!(session.world().state_rev, round_rev.unwrap());
        assert_eq!(session.world().state_rev, mark.state_rev);
        assert!(root.join("state/journal/bundle-0000.jsonl").is_file());
        assert!(!root.join("state/journal/bundle-0000.jsonl.gz").is_file());

        let mut again = session.start_round();
        again.round_index = 2;
        commit_round(&mut session, again, round_line(2)).unwrap();
        let bundles = crate::store::journal::read_bundle_records(&root).unwrap();
        assert_eq!(bundles.len(), 2, "{bundles:?}");
        for (bundle, entries) in &bundles {
            let checkpoints = entries
                .iter()
                .filter(|entry| matches!(entry, crate::store::records::Record::Checkpoint(_)))
                .count();
            assert_eq!(checkpoints, 1, "{bundle}");
        }
        assert!(root.join("state/journal/bundle-0000.jsonl.gz").is_file());
        assert!(!root.join("state/journal/bundle-0000.jsonl").exists());
        assert!(root.join("state/journal/bundle-0001.jsonl.gz").is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_save_stays_committed_and_the_next_commit_starts_a_bundle() {
        let root = copy_pack("docks", "save-fail");
        let _clear = FailGuard;
        set_checkpoint_every(&root, 2);
        let mut session = crate::testing::resume(&root);
        let mut first = session.start_round();
        first.round_index = 1;
        commit_round(&mut session, first, round_line(1)).unwrap();

        let mut second = session.start_round();
        second.round_index = 2;
        durability::fail_next("save.yaml.tmp");
        commit_round(&mut session, second, round_line(2)).unwrap();
        let live = crate::store::journal::read_live_records(&root).unwrap();
        let mark = crate::store::journal::latest_checkpoint(&live).unwrap();
        assert_eq!(session.world().state_rev, mark.state_rev);

        let mut third = session.start_round();
        third.round_index = 3;
        commit_round(&mut session, third, round_line(3)).unwrap();
        let bundles = crate::store::journal::read_bundle_records(&root).unwrap();
        assert_eq!(bundles.len(), 2, "{bundles:?}");
        let (first_bundle, first_entries) = &bundles[0];
        let (second_bundle, second_entries) = &bundles[1];
        assert_eq!(count_checkpoints(first_entries), 1, "{first_bundle}");
        assert_eq!(count_checkpoints(second_entries), 0, "{second_bundle}");
        assert!(second_entries.iter().any(|entry| {
            matches!(entry, crate::store::records::Record::Round(record) if record.round_index == 3)
        }));
        let saved = fs::read(root.join("state/save.yaml")).unwrap();
        let kept = crate::store::journal::latest_checkpoint(first_entries).unwrap();
        assert_eq!(crate::hash::blake2s_tagged(&saved), kept.state_hash);
        let snap = crate::store::journal::checkpoint_path(&root, kept.state_rev);
        assert_eq!(fs::read(snap).unwrap(), saved);
        let _ = fs::remove_dir_all(&root);
    }

    fn count_checkpoints(entries: &[crate::store::records::Record]) -> usize {
        entries
            .iter()
            .filter(|entry| matches!(entry, crate::store::records::Record::Checkpoint(_)))
            .count()
    }

    struct FailGuard;

    impl Drop for FailGuard {
        fn drop(&mut self) {
            durability::clear_fail();
        }
    }

    fn set_checkpoint_every(root: &Path, every: u64) {
        let path = root.join("pack.yaml");
        let mut text = fs::read_to_string(&path).unwrap();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("checkpoint_every: {every}\n"));
        fs::write(path, text).unwrap();
    }

    fn round_line(round_index: u64) -> RoundRecord {
        RoundRecord {
            round_index,
            rules_rev: crate::store::RULES_REV,
            ..Default::default()
        }
    }
}
