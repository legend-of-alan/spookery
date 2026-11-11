//! Publish a round or a migrate: the journal record, then at a checkpoint the save.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::canon::{pack_rev, Canon};
use crate::diagnostic::first_reason;
use crate::hash::blake2s_tagged;
use crate::state::{sort_unique, State};
use crate::store::durability::{fsync_dir, fsync_file, refuse_io, write_atomic};
use crate::store::journal;
use crate::store::lock::{lock_write, PackGuard, WriteGuard};
use crate::store::records::{MigrateRecord, Record, RoundRecord};
use crate::store::recovery::close_finished;
use crate::store::save::{self, check_save, find_missing_meters};
use crate::world::World;
use crate::Reason;

const CHECKPOINT_WARNING: &str = "the checkpoint bundle was not closed";

pub struct CommitOutcome {
    pub state_rev: u64,
    pub log_path: String,
    /// The committed round's index and the world time it ended at.
    pub round_index: u64,
    pub world_time: u64,
}

/// Seal one played round, append it, finish a due checkpoint, and clear the queue. `state` is
/// the round's working copy of the save, published only once the round is on disk.
pub fn commit(
    world: &mut World,
    mut state: State,
    record: RoundRecord,
) -> Result<CommitOutcome, Reason> {
    let write = lock_write(&world.root, &world.lock)?;
    settle_checkpoint(world, &write)?;
    bump_state_rev(&mut state)?;
    let now_bytes = save::emit(&state)?.into_bytes();
    let line = seal_round(
        record,
        world.state.state_rev,
        &mut state,
        &now_bytes,
        &world.root,
    )?;
    let due = is_checkpoint_due(world.canon().checkpoint_every, state.round_index);
    let (payload, chain) = if due {
        let (payload, chain) = seal_checkpoint(line, &state, &now_bytes)?;
        (payload, Some(chain))
    } else {
        (line, None)
    };
    let mut log_path = append_to_bundle(&world.root, &payload, &write)?;
    state.accept();
    world.state = state;
    world.queue.clear();
    if let Some(chain) = chain {
        if finish_checkpoint(world, &now_bytes, &chain, &write) {
            if !log_path.ends_with(".gz") {
                log_path.push_str(".gz");
            }
        } else {
            world.set_warning(CHECKPOINT_WARNING);
        }
    }
    Ok(CommitOutcome {
        state_rev: world.state.state_rev,
        log_path,
        round_index: world.state.round_index,
        world_time: world.state.world_time,
    })
}

/// The journal already has this round. Keep the new state in memory.
pub fn keep_in_memory(world: &mut World<()>, mut state: State) -> Result<(), Reason> {
    bump_state_rev(&mut state)?;
    state.accept();
    world.state = state;
    Ok(())
}

fn bump_state_rev(state: &mut State) -> Result<(), Reason> {
    state.state_rev = state.state_rev.checked_add(1).ok_or(Reason::OutOfRange)?;
    Ok(())
}

/// Seal a migrate record for `state`, which is already the journal tip or the on-disk save.
/// `edited` means `state` is a hand edit of the checkpoint head. The journal does not hold
/// that edit, so a checkpoint of the migrated save follows the record and replay restarts
/// from it. The caller holds the pack lock.
pub fn commit_migrate(
    root: &Path,
    canon: &Canon,
    state: State,
    edited: bool,
    lock: &PackGuard,
) -> Result<(), Reason> {
    let pack_rev_after = pack_rev(root)?;
    let pack_rev_before = state.pack_rev.clone();
    let state_rev_before = state.state_rev;
    let state_rev_after = state_rev_before.checked_add(1).ok_or(Reason::OutOfRange)?;
    let stale = save::find_stale_modifiers(canon, &state).into_iter();
    let mut dropped_modifiers: Vec<String> = stale.map(|(_, id)| id).collect();
    sort_unique(&mut dropped_modifiers);
    let mut record = MigrateRecord {
        pack_rev_before,
        pack_rev_after,
        state_rev_before,
        state_rev_after,
        state_hash: String::new(),
        meters: find_missing_meters(canon, &state),
        dropped_modifiers,
        dropped_meters: save::find_stale_meters(canon, &state),
        edited,
        prev: String::new(),
        chain: String::new(),
    };
    let state = save::migrate(canon, state, &record)?;
    check_save(canon, &state).map_err(|diags| first_reason(&diags))?;
    let emitted = save::emit(&state)?.into_bytes();
    record.state_hash = blake2s_tagged(&emitted);
    let entry = Record::Migrate(record);
    let write = lock_write(root, lock)?;
    let prev = journal::read_chain_tip(root).map_err(|fault| fault.reason)?;
    let line = journal::seal(&entry, &prev);
    if !edited {
        append_to_bundle(root, &line, &write)?;
        return Ok(());
    }
    // The snapshot lands before the record, so a crash after the append still replays.
    write_atomic(&journal::checkpoint_path(root, state.state_rev), &emitted)?;
    let (payload, chain) = seal_checkpoint(line, &state, &emitted)?;
    append_to_bundle(root, &payload, &write)?;
    store_checkpoint(
        root,
        &emitted,
        state.state_rev,
        state.round_index,
        &chain,
        &write,
    )?;
    // The migrate is committed. A bundle left open is closed by the next append.
    let _ = close_finished(root, &write);
    Ok(())
}

/// Follow a sealed `line` with a checkpoint of `state`. Returns both lines and the
/// checkpoint's chain.
fn seal_checkpoint(
    line: String,
    state: &State,
    now_bytes: &[u8],
) -> Result<(String, String), Reason> {
    let prev = journal::sealed_chain(&line).map_err(|fault| fault.reason)?;
    let mark = journal::checkpoint_record(
        state.round_index,
        state.state_rev,
        &blake2s_tagged(now_bytes),
    );
    let sealed = journal::seal(&mark, &prev);
    let chain = journal::sealed_chain(&sealed).map_err(|fault| fault.reason)?;
    Ok((format!("{line}{sealed}"), chain))
}

fn seal_round(
    mut record: RoundRecord,
    state_rev_before: u64,
    state: &mut State,
    now_bytes: &[u8],
    root: &Path,
) -> Result<String, Reason> {
    record.state_rev_before = state_rev_before;
    record.state_rev_after = state.state_rev;
    record.memories = state.take_memories();
    record.state_hash = blake2s_tagged(now_bytes);
    let prev = journal::read_chain_tip(root).map_err(|fault| fault.reason)?;
    Ok(journal::seal(&Record::Round(record), &prev))
}

fn is_checkpoint_due(every: u64, round_index: u64) -> bool {
    every > 0 && round_index > 0 && round_index.is_multiple_of(every)
}

fn append_to_bundle(root: &Path, payload: &str, write: &WriteGuard) -> Result<String, Reason> {
    // A failed close must not refuse this round or append into the closed bundle.
    let _ = close_finished(root, write);
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
    // The bundle path is built under `root`.
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
    let head = serde_json::to_string(&journal::CheckpointHead {
        rules_rev: crate::store::RULES_REV,
        state_rev,
        round_index,
        state_hash: blake2s_tagged(now_bytes),
        chain: chain.to_string(),
    })
    .map_err(|_| Reason::Schema)?;
    write_atomic(&journal::head_path(root), head.as_bytes())?;
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

/// The journal line is already durable. `false` means `save.yaml` or the gzip did
/// not finish. The round stays committed either way.
fn finish_checkpoint(world: &mut World, now_bytes: &[u8], chain: &str, write: &WriteGuard) -> bool {
    let (rev, round) = (world.state.state_rev, world.state.round_index);
    store_checkpoint(&world.root, now_bytes, rev, round, chain, write).is_ok()
        && close_finished(&world.root, write).is_ok()
}

/// Publish `save.yaml` and the snapshot for a checkpoint that is already in the
/// journal, then close that bundle. The next round can then start a new file.
pub(crate) fn settle_checkpoint<L>(world: &World<L>, write: &WriteGuard) -> Result<(), Reason> {
    let Some(path) = journal::find_live_bundle(&world.root) else {
        return Ok(());
    };
    let Some(mark) = journal::trailing_checkpoint(&path) else {
        return Ok(());
    };
    let now_bytes = save::emit(world.state())?.into_bytes();
    if blake2s_tagged(&now_bytes) != mark.state_hash {
        return Ok(());
    }
    let save = journal::save_path(&world.root);
    let snap = journal::checkpoint_path(&world.root, mark.state_rev);
    let save_ok = journal::hashes_to(&save, &mark.state_hash);
    let snap_ok = journal::hashes_to(&snap, &mark.state_hash);
    if !save_ok || !snap_ok {
        store_checkpoint(
            &world.root,
            &now_bytes,
            mark.state_rev,
            mark.round_index,
            &mark.chain,
            write,
        )?;
    }
    let _ = close_finished(&world.root, write);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::state::MemoryLine;
    use crate::store::durability;
    use crate::store::lock::lock_pack;
    use crate::store::records::RoundRecord;
    use crate::testing::copy_docks;
    use crate::world::World;

    use super::commit;

    #[test]
    fn a_reopen_keeps_every_journal_record_the_commit_wrote() {
        let root = copy_docks("journal-reopen");
        let mut world = crate::testing::resume(&root);
        let pre = fs::read(root.join("state/save.yaml")).unwrap();
        let mut state = world.begin_round();
        state.push_memory(MemoryLine {
            round_index: 0,
            actor_id: "mira".into(),
            intent_id: "rest".to_string(),
            fact_ids: vec![],
            tags: vec![],
            text: Some("kept".to_string()),
        });
        let line = round_line(0);
        commit(&mut world, state, line).unwrap();
        assert_no_tmp(&root);
        let bundle = root.join("state/journal/bundle-0000.jsonl");
        let bundle_bytes = fs::read(&bundle).unwrap();
        assert_eq!(fs::read(root.join("state/save.yaml")).unwrap(), pre);
        let bundle_text = String::from_utf8(bundle_bytes.clone()).unwrap();
        assert!(
            bundle_text.contains("\"state_rev_after\":1"),
            "{bundle_text}"
        );
        assert!(
            bundle_text.contains("\"chain\":\"blake2s:"),
            "{bundle_text}"
        );
        assert!(bundle_text.contains("kept"), "{bundle_text}");
        let next = world.begin_round();
        commit(&mut world, next, round_line(1)).unwrap();
        let bundle_bytes = fs::read(&bundle).unwrap();
        drop(world);
        drop(lock_pack(&root).unwrap());
        let opened = World::open(&root).unwrap();
        assert_eq!(opened.state().state_rev, 0);
        assert_eq!(fs::read(&bundle).unwrap(), bundle_bytes);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_gzip_keeps_the_round_and_the_next_commit_starts_a_bundle() {
        let root = copy_docks("gzip-fail");
        let _clear = FailGuard;
        set_checkpoint_every(&root, 1);
        let mut world = crate::testing::resume(&root);
        let mut state = world.begin_round();
        state.round_index = 1;
        durability::fail_next(".jsonl.gz.tmp");
        commit(&mut world, state, round_line(1)).unwrap();
        let live = crate::store::journal::read_live_records(&root).unwrap();
        let round_rev = live.iter().rev().find_map(|entry| match entry {
            crate::store::records::Record::Round(record) => Some(record.state_rev_after),
            _ => None,
        });
        let mark = crate::store::journal::latest_checkpoint(&live).unwrap();
        assert_eq!(world.state().state_rev, round_rev.unwrap());
        assert_eq!(world.state().state_rev, mark.state_rev);
        assert_eq!(world.take_warning(), Some(super::CHECKPOINT_WARNING));
        assert!(root.join("state/journal/bundle-0000.jsonl").is_file());
        assert!(!root.join("state/journal/bundle-0000.jsonl.gz").is_file());

        let mut again = world.begin_round();
        again.round_index = 2;
        commit(&mut world, again, round_line(2)).unwrap();
        assert!(world.take_warning().is_none());
        let bundles = crate::store::journal::read_bundle_records(&root).unwrap();
        assert_eq!(bundles.len(), 2, "{bundles:?}");
        for (name, entries) in &bundles {
            let checkpoints = entries
                .iter()
                .filter(|entry| matches!(entry, crate::store::records::Record::Checkpoint(_)))
                .count();
            assert_eq!(checkpoints, 1, "{name}");
        }
        assert!(root.join("state/journal/bundle-0000.jsonl.gz").is_file());
        assert!(!root.join("state/journal/bundle-0000.jsonl").exists());
        assert!(root.join("state/journal/bundle-0001.jsonl.gz").is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_save_stays_committed_and_the_next_commit_starts_a_bundle() {
        let root = copy_docks("save-fail");
        let _clear = FailGuard;
        set_checkpoint_every(&root, 2);
        let mut world = crate::testing::resume(&root);
        let mut first = world.begin_round();
        first.round_index = 1;
        commit(&mut world, first, round_line(1)).unwrap();

        let mut second = world.begin_round();
        second.round_index = 2;
        durability::fail_next("save.yaml.tmp");
        commit(&mut world, second, round_line(2)).unwrap();
        let live = crate::store::journal::read_live_records(&root).unwrap();
        let mark = crate::store::journal::latest_checkpoint(&live).unwrap();
        assert_eq!(world.state().state_rev, mark.state_rev);
        assert_eq!(world.take_warning(), Some(super::CHECKPOINT_WARNING));

        let mut third = world.begin_round();
        third.round_index = 3;
        commit(&mut world, third, round_line(3)).unwrap();
        let bundles = crate::store::journal::read_bundle_records(&root).unwrap();
        assert_eq!(bundles.len(), 2, "{bundles:?}");
        let (first_name, first_entries) = &bundles[0];
        let (second_name, second_entries) = &bundles[1];
        assert_eq!(count_checkpoints(first_entries), 1, "{first_name}");
        assert_eq!(count_checkpoints(second_entries), 0, "{second_name}");
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

    fn assert_no_tmp(dir: &Path) {
        let mut pending = vec![dir.to_path_buf()];
        while let Some(current) = pending.pop() {
            for entry in fs::read_dir(&current).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy();
                assert!(!name.ends_with(".tmp"), "{}", path.display());
            }
        }
    }

    fn round_line(round_index: u64) -> RoundRecord {
        RoundRecord {
            round_index,
            rules_rev: crate::store::RULES_REV,
            ..Default::default()
        }
    }
}
