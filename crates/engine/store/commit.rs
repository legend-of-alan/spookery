//! Publish one round: the journal record, then the save.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::canon::{check_save, migrate_save, pack_rev, pending_meters};
use crate::diagnostic::reason_of;
use crate::draft::Draft;
use crate::error::Reason;
use crate::journal::{self, hash_now};
use crate::model::{MemoryLine, Now};
use crate::round::log::RoundLine;
use crate::store::codec::{canonicalize, emit_memory_line, emit_save};
pub use crate::store::durable::atomic_publish;
use crate::store::durable::{fsync_dir, fsync_file};
use crate::store::lock::{write_guard, LockGuard, WriteGuard};
use crate::store::recover::close_finished;
use crate::world::World;

const CHECKPOINT_WARNING: &str = "the checkpoint bundle was not closed";

pub struct CommitOutcome {
    pub state_rev: u64,
    pub log_path: String,
}

/// Seal one played round, append it, finish a due checkpoint, and clear the queue.
pub fn commit(
    world: &mut World,
    mut draft: Draft,
    line: RoundLine,
) -> Result<CommitOutcome, Reason> {
    let write = write_guard(&world.root, &world.lock)?;
    settle(world, &write)?;
    bump_state_rev(&mut draft)?;
    let now_bytes = emit_save(&draft.state)?.into_bytes();
    let line = seal(line, world.now.state_rev, &draft, &now_bytes, &world.root)?;
    let due = checkpoint_due(world.canon().checkpoint_every, draft.state.round_index);
    let (payload, chain) = if due {
        let (payload, chain) = seal_checkpoint(line, &draft.state, &now_bytes)?;
        (payload, Some(chain))
    } else {
        (line, None)
    };
    let mut log_path = append_bundle(&world.root, &payload, &write)?;
    world.now = draft.state;
    world.queue.clear();
    if let Some(chain) = chain {
        if finish_checkpoint(world, &now_bytes, &chain, &write) {
            if !log_path.ends_with(".gz") {
                log_path.push_str(".gz");
            }
        } else {
            world.note_warning(CHECKPOINT_WARNING);
        }
    }
    Ok(CommitOutcome {
        state_rev: world.now.state_rev,
        log_path,
    })
}

/// The journal already has this round. Keep the new state in memory.
pub fn keep_in_memory(world: &mut World<()>, mut draft: Draft) -> Result<(), Reason> {
    bump_state_rev(&mut draft)?;
    world.now = draft.state;
    Ok(())
}

fn bump_state_rev(draft: &mut Draft) -> Result<(), Reason> {
    draft.state.state_rev = draft
        .state
        .state_rev
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    canonicalize(&mut draft.state);
    Ok(())
}

/// Seal a migrate record for `now`, which is already the journal tip or the on-disk save.
/// `edited` means `now` is a hand edit of the checkpoint head. The journal does not hold
/// that edit, so a checkpoint of the migrated save follows the record and replay restarts
/// from it. The caller holds the pack lock.
pub fn migrate_journal(
    root: &Path,
    now: Now,
    edited: bool,
    lock: &LockGuard,
) -> Result<(), Reason> {
    if !root.join("pack.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    let canon = crate::canon::load_canon(root).map_err(reason_of)?;
    let pack_rev_after = pack_rev(root)?;
    let pack_rev_before = now.pack_rev.clone();
    let state_rev_before = now.state_rev;
    let state_rev_after = state_rev_before
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    let added = pending_meters(&canon, &now);
    let now = migrate_save(now, &pack_rev_after, state_rev_after, &added);
    check_save(&canon, &now).map_err(reason_of)?;
    let emitted = emit_save(&now)?.into_bytes();
    let entry = journal::migrate_entry(
        &pack_rev_before,
        &now.pack_rev,
        state_rev_before,
        now.state_rev,
        &hash_now(&emitted),
        &added,
        edited,
    );
    let write = write_guard(root, lock)?;
    let prev = journal::read_chain_tip(root).map_err(|_| Reason::Schema)?;
    let line = journal::seal(&entry, &prev);
    if !edited {
        append_bundle(root, &line, &write)?;
        return Ok(());
    }
    // The snapshot lands before the record, so a crash after the append still replays.
    atomic_publish(&journal::checkpoint_path(root, now.state_rev), &emitted)?;
    let (payload, chain) = seal_checkpoint(line, &now, &emitted)?;
    append_bundle(root, &payload, &write)?;
    store_checkpoint(
        root,
        &emitted,
        now.state_rev,
        now.round_index,
        &chain,
        &write,
    )?;
    // The migrate is committed. A bundle left open is closed by the next append.
    let _ = close_finished(root, &write);
    Ok(())
}

/// Follow a sealed `line` with a checkpoint of `now`. Returns both lines and the checkpoint's chain.
fn seal_checkpoint(line: String, now: &Now, now_bytes: &[u8]) -> Result<(String, String), Reason> {
    let prev = journal::sealed_chain(&line).map_err(|_| Reason::Schema)?;
    let mark = journal::checkpoint_entry(now.round_index, now.state_rev, &hash_now(now_bytes));
    let sealed = journal::seal(&mark, &prev);
    let chain = journal::sealed_chain(&sealed).map_err(|_| Reason::Schema)?;
    Ok((format!("{line}{sealed}"), chain))
}

fn seal(
    line: RoundLine,
    state_rev_before: u64,
    draft: &Draft,
    now_bytes: &[u8],
    root: &Path,
) -> Result<String, Reason> {
    let record = journal::RoundRecord {
        round_index: line.clock.round_index,
        rules_rev: crate::store::RULES_REV,
        state_rev_before,
        state_rev_after: draft.state.state_rev,
        world_time_before: line.clock.world_time_before,
        world_time_after: line.clock.world_time_after,
        presence_humans: line.presence.humans,
        actors_present: line.presence.actors_present,
        seed: line.seed,
        last_round_at: line.wall,
        intents: line.queue,
        rejected: line.log.rejected,
        effects: line.log.effects,
        arrivals: line.log.arrivals,
        fast_forward: line.fast_forward,
        dice: line.log.dice,
        steps_run: line.log.steps,
        memories: draft.memories.clone(),
        now_hash: hash_now(now_bytes),
        prev: String::new(),
        chain: String::new(),
    };
    let prev = journal::read_chain_tip(root).map_err(|_| Reason::Schema)?;
    Ok(journal::seal(&journal::JournalEntry::Round(record), &prev))
}

fn checkpoint_due(every: u64, round_index: u64) -> bool {
    every > 0 && round_index > 0 && round_index.is_multiple_of(every)
}

pub fn append_bundle(root: &Path, payload: &str, write: &WriteGuard) -> Result<String, Reason> {
    // A failed close must not refuse this round or append into the closed bundle.
    let _ = close_finished(root, write);
    let path = journal::next_bundle_path(root);
    let created = !path.is_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| Reason::CommitConflict)?;
    }
    {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|_| Reason::CommitConflict)?;
        file.write_all(payload.as_bytes())
            .map_err(|_| Reason::CommitConflict)?;
    }
    fsync_file(&path)?;
    if created {
        if let Some(parent) = path.parent() {
            fsync_dir(parent)?;
        }
    }
    Ok(path
        .strip_prefix(root)
        .map_err(|_| Reason::CommitConflict)?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn store_checkpoint(
    root: &Path,
    now_bytes: &[u8],
    state_rev: u64,
    round_index: u64,
    chain: &str,
    write: &WriteGuard,
) -> Result<(), Reason> {
    let head = serde_json::to_string(&journal::BundleHead {
        rules_rev: crate::store::RULES_REV,
        state_rev,
        round_index,
        state_hash: hash_now(now_bytes),
        chain: chain.to_string(),
    })
    .map_err(|_| Reason::Schema)?;
    atomic_publish(
        &root.join("state").join(journal::head_rel()),
        head.as_bytes(),
    )?;
    snapshot_memories(root, write)?;
    let state = root.join("state");
    let save = state.join("save.yaml");
    let base = state.join(journal::base_rel());
    if !base.is_file() {
        fs::copy(&save, &base).map_err(|_| Reason::CommitConflict)?;
        fsync_file(&base)?;
        if let Some(parent) = base.parent() {
            fsync_dir(parent)?;
        }
    }
    let snap_dir = state.join("checkpoints");
    fs::create_dir_all(&snap_dir).map_err(|_| Reason::CommitConflict)?;
    atomic_publish(&save, now_bytes)?;
    atomic_publish(&snap_dir.join(format!("{state_rev}.yaml")), now_bytes)?;
    Ok(())
}

/// The journal line is already durable. `false` means `save.yaml` or the gzip did
/// not finish. The round stays committed either way.
fn finish_checkpoint(world: &mut World, now_bytes: &[u8], chain: &str, write: &WriteGuard) -> bool {
    let (rev, round) = (world.now.state_rev, world.now.round_index);
    store_checkpoint(&world.root, now_bytes, rev, round, chain, write).is_ok()
        && close_finished(&world.root, write).is_ok()
}

/// Publish `save.yaml` and the snapshot for a checkpoint that is already in the
/// journal, then close that bundle. The next round can then start a new file.
pub fn settle_open_checkpoint<L>(world: &World<L>, lock: &LockGuard) -> Result<(), Reason> {
    settle(world, &write_guard(&world.root, lock)?)
}

fn settle<L>(world: &World<L>, write: &WriteGuard) -> Result<(), Reason> {
    let Some(path) = journal::find_live_bundle(&world.root) else {
        return Ok(());
    };
    let Some(mark) = journal::trailing_checkpoint(&path) else {
        return Ok(());
    };
    let now_bytes = emit_save(world.now())?.into_bytes();
    if hash_now(&now_bytes) != mark.state_hash {
        return Ok(());
    }
    let save = world.root.join("state/save.yaml");
    let snap = journal::checkpoint_path(&world.root, mark.state_rev);
    let save_ok = fs::read(&save)
        .ok()
        .is_some_and(|bytes| hash_now(&bytes) == mark.state_hash);
    let snap_ok = fs::read(&snap)
        .ok()
        .is_some_and(|bytes| hash_now(&bytes) == mark.state_hash);
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

fn snapshot_memories(root: &Path, _write: &WriteGuard) -> Result<(), Reason> {
    let entries = journal::read_live(root).map_err(|_| Reason::Schema)?;
    let mut lines = Vec::new();
    for entry in entries {
        if let journal::JournalEntry::Round(record) = entry {
            lines.extend(record.memories);
        }
    }
    for (actor, body) in memory_bodies(&root.join("state"), &lines)? {
        let path = root.join("state/memories").join(format!("{actor}.jsonl"));
        atomic_publish(&path, body.as_bytes())?;
    }
    Ok(())
}

fn memory_bodies(state: &Path, lines: &[MemoryLine]) -> Result<BTreeMap<String, String>, Reason> {
    let mut by_actor: BTreeMap<String, Vec<&MemoryLine>> = BTreeMap::new();
    for line in lines {
        by_actor
            .entry(line.actor_id.clone())
            .or_default()
            .push(line);
    }
    let mut out = BTreeMap::new();
    for (actor, fresh) in by_actor {
        let path = state.join("memories").join(format!("{actor}.jsonl"));
        let mut body = if path.is_file() {
            fs::read_to_string(&path).map_err(|_| Reason::CommitConflict)?
        } else {
            String::new()
        };
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        let mut seen: BTreeSet<String> = body.lines().map(str::to_string).collect();
        for line in fresh {
            let text = emit_memory_line(line);
            if seen.contains(&text) {
                continue;
            }
            body.push_str(&text);
            body.push('\n');
            seen.insert(text);
        }
        out.insert(actor, body);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::draft::Draft;
    use crate::model::{MemoryLine, Presence};
    use crate::round::log::{RoundLine, RoundLog};
    use crate::round::stage::advance::Clock;
    use crate::store::durable::{self, Mark};
    use crate::store::lock::lock_pack;
    use crate::testing::temp_pack;
    use crate::world::World;

    use super::commit;

    #[test]
    fn a_reopen_keeps_every_journal_record_the_commit_wrote() {
        let root = temp_pack("journal-reopen");
        let mut world = crate::testing::open(&root);
        let pre = fs::read(root.join("state/save.yaml")).unwrap();
        let mut draft = Draft::begin(&world);
        draft.memories.push(MemoryLine {
            round_index: 0,
            actor_id: "mira".to_string(),
            intent_id: "rest".to_string(),
            fact_ids: vec![],
            tags: vec![],
            text: Some("kept".to_string()),
        });
        let line = journal_line(0);
        durable::start_trace();
        commit(&mut world, draft, line).unwrap();
        let marks = durable::take_trace();
        // A new bundle is one file flush plus the directory flush.
        assert_eq!(
            labels(&marks),
            ["file bundle", "dir journal"],
            "{}",
            show(&marks)
        );
        assert!(!root.join("state/.txn").exists());
        assert_no_tmp(&root);
        let bundle = root.join("state/journal/bundle-0000.jsonl");
        let memory = root.join("state/memories/mira.jsonl");
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
        assert!(!memory.exists());
        drop(world);
        drop(lock_pack(&root).unwrap());
        let opened = World::open(&root).unwrap();
        assert_eq!(opened.now().state_rev, 0);
        assert_eq!(fs::read(&bundle).unwrap(), bundle_bytes);
        assert!(fs::read_to_string(&memory).unwrap().contains("kept"));
        assert!(!root.join("state/.txn").exists());

        let mut again = crate::testing::open(&root);
        let next = Draft::begin(&again);
        durable::start_trace();
        commit(&mut again, next, journal_line(1)).unwrap();
        let marks = durable::take_trace();
        assert_eq!(labels(&marks), ["file bundle"], "{}", show(&marks));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_gzip_keeps_the_round_and_the_next_commit_starts_a_bundle() {
        let root = temp_pack("gzip-fail");
        let _clear = FailGuard;
        set_checkpoint_every(&root, 1);
        let mut world = crate::testing::open(&root);
        let mut draft = Draft::begin(&world);
        draft.state.round_index = 1;
        durable::fail_next(".jsonl.gz.tmp");
        commit(&mut world, draft, journal_line(1)).unwrap();
        let live = crate::journal::read_live(&root).unwrap();
        let round_rev = live.iter().rev().find_map(|entry| match entry {
            crate::journal::JournalEntry::Round(record) => Some(record.state_rev_after),
            _ => None,
        });
        let mark = crate::journal::latest_checkpoint(&live).unwrap();
        assert_eq!(world.now().state_rev, round_rev.unwrap());
        assert_eq!(world.now().state_rev, mark.state_rev);
        assert_eq!(world.take_warning(), Some(super::CHECKPOINT_WARNING));
        assert!(root.join("state/journal/bundle-0000.jsonl").is_file());
        assert!(!root.join("state/journal/bundle-0000.jsonl.gz").is_file());

        let mut again = Draft::begin(&world);
        again.state.round_index = 2;
        commit(&mut world, again, journal_line(2)).unwrap();
        assert!(world.take_warning().is_none());
        let bundles = crate::journal::bundle_entries(&root).unwrap();
        assert_eq!(bundles.len(), 2, "{bundles:?}");
        for (name, entries) in &bundles {
            let checkpoints = entries
                .iter()
                .filter(|entry| matches!(entry, crate::journal::JournalEntry::Checkpoint(_)))
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
        let root = temp_pack("save-fail");
        let _clear = FailGuard;
        set_checkpoint_every(&root, 2);
        let mut world = crate::testing::open(&root);
        let mut first = Draft::begin(&world);
        first.state.round_index = 1;
        commit(&mut world, first, journal_line(1)).unwrap();

        let mut second = Draft::begin(&world);
        second.state.round_index = 2;
        durable::fail_next("save.yaml.tmp");
        commit(&mut world, second, journal_line(2)).unwrap();
        let live = crate::journal::read_live(&root).unwrap();
        let mark = crate::journal::latest_checkpoint(&live).unwrap();
        assert_eq!(world.now().state_rev, mark.state_rev);
        assert_eq!(world.take_warning(), Some(super::CHECKPOINT_WARNING));

        let mut third = Draft::begin(&world);
        third.state.round_index = 3;
        commit(&mut world, third, journal_line(3)).unwrap();
        let bundles = crate::journal::bundle_entries(&root).unwrap();
        assert_eq!(bundles.len(), 2, "{bundles:?}");
        let (first_name, first_entries) = &bundles[0];
        let (second_name, second_entries) = &bundles[1];
        assert_eq!(count_checkpoints(first_entries), 1, "{first_name}");
        assert_eq!(count_checkpoints(second_entries), 0, "{second_name}");
        assert!(second_entries.iter().any(|entry| {
            matches!(entry, crate::journal::JournalEntry::Round(record) if record.round_index == 3)
        }));
        let saved = fs::read(root.join("state/save.yaml")).unwrap();
        let kept = crate::journal::latest_checkpoint(first_entries).unwrap();
        assert_eq!(crate::journal::hash_now(&saved), kept.state_hash);
        let snap = crate::journal::checkpoint_path(&root, kept.state_rev);
        assert_eq!(fs::read(snap).unwrap(), saved);
        let _ = fs::remove_dir_all(&root);
    }

    fn count_checkpoints(entries: &[crate::journal::JournalEntry]) -> usize {
        entries
            .iter()
            .filter(|entry| matches!(entry, crate::journal::JournalEntry::Checkpoint(_)))
            .count()
    }

    struct FailGuard;

    impl Drop for FailGuard {
        fn drop(&mut self) {
            durable::clear_fail();
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

    fn labels(marks: &[Mark]) -> Vec<String> {
        marks.iter().map(label).collect()
    }

    fn label(mark: &Mark) -> String {
        let path = match mark {
            Mark::File(path) | Mark::Dir(path) | Mark::Rename(path) => path.as_path(),
        };
        let name = path
            .file_name()
            .and_then(|file| file.to_str())
            .unwrap_or("");
        match mark {
            Mark::File(_) if name.ends_with(".tmp") => tmp_label(name, path),
            Mark::File(_) if name == "COMPLETE" && parent_name(path, ".txn") => {
                "file complete".to_string()
            }
            Mark::File(_) if name == "save.yaml" && parent_name(path, "prev") => {
                "file save".to_string()
            }
            Mark::Dir(_) if name == "prev" && parent_name(path, ".txn") => "dir prev".to_string(),
            Mark::Dir(_) if name == ".txn" => "dir txn".to_string(),
            Mark::File(_) if name.starts_with("bundle-") && parent_name(path, "journal") => {
                "file bundle".to_string()
            }
            Mark::Dir(_) if name == "journal" && parent_name(path, "state") => {
                "dir journal".to_string()
            }
            Mark::Dir(_) if name == "memories" && parent_name(path, "state") => {
                "dir memories".to_string()
            }
            Mark::Dir(_) if name == "state" => "dir state".to_string(),
            Mark::Rename(_) if name == "base.yaml" && parent_name(path, "journal") => {
                "rename base".to_string()
            }
            Mark::Rename(_) if name.starts_with("tick-") => "rename tick".to_string(),
            Mark::Rename(_) if parent_name(path, "memories") => "rename memory".to_string(),
            Mark::Rename(_) if name == "save.yaml" && parent_name(path, "state") => {
                "rename save".to_string()
            }
            _ => format!("unexpected {}", path.display()),
        }
    }

    fn tmp_label(name: &str, path: &Path) -> String {
        if name.starts_with("save.yaml") {
            "file save.tmp".to_string()
        } else if name.starts_with("base.yaml") {
            "file base.tmp".to_string()
        } else if name.starts_with("tick-") {
            "file tick.tmp".to_string()
        } else if parent_name(path, "memories") {
            "file memory.tmp".to_string()
        } else {
            format!("unexpected {}", path.display())
        }
    }

    fn parent_name(path: &Path, name: &str) -> bool {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|file| file.to_str())
            == Some(name)
    }

    fn show(marks: &[Mark]) -> String {
        marks
            .iter()
            .map(|mark| match mark {
                Mark::File(path) => format!("file {}", path.display()),
                Mark::Dir(path) => format!("dir {}", path.display()),
                Mark::Rename(path) => format!("rename {}", path.display()),
            })
            .collect::<Vec<_>>()
            .join("\n")
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

    fn journal_line(round_index: u64) -> RoundLine {
        RoundLine {
            clock: Clock {
                round_index,
                world_time_before: 0,
                world_time_after: 0,
            },
            presence: Presence::default(),
            seed: String::new(),
            wall: String::new(),
            fast_forward: false,
            log: RoundLog::default(),
            queue: Vec::new(),
        }
    }
}
