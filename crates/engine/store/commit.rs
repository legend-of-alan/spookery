//! Publish one round: the journal record, then the save.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::canon::{check_save, migrate_save, missing_meters, pack_rev};
use crate::diagnostic::first_reason;
use crate::draft::Draft;
use crate::error::Reason;
use crate::journal::{self, blake2s_tagged};
use crate::model::{MemoryLine, State};
use crate::round::log::PlayedRound;
use crate::store::codec::{emit_memory_line, emit_save, normalize};
pub use crate::store::durability::write_atomic;
use crate::store::durability::{fsync_dir, fsync_file};
use crate::store::lock::{lock_write, PackGuard, WriteGuard};
use crate::store::recovery::close_finished;
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
    line: PlayedRound,
) -> Result<CommitOutcome, Reason> {
    let write = lock_write(&world.root, &world.lock)?;
    finish_trailing_checkpoint(world, &write)?;
    bump_state_rev(&mut draft)?;
    let now_bytes = emit_save(&draft.state)?.into_bytes();
    let line = seal_round(line, world.state.state_rev, &draft, &now_bytes, &world.root)?;
    let due = is_checkpoint_due(world.canon().checkpoint_every, draft.state.round_index);
    let (payload, chain) = if due {
        let (payload, chain) = seal_checkpoint(line, &draft.state, &now_bytes)?;
        (payload, Some(chain))
    } else {
        (line, None)
    };
    let mut log_path = append_to_bundle(&world.root, &payload, &write)?;
    world.state = draft.state;
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
    })
}

/// The journal already has this round. Keep the new state in memory.
pub fn keep_in_memory(world: &mut World<()>, mut draft: Draft) -> Result<(), Reason> {
    bump_state_rev(&mut draft)?;
    world.state = draft.state;
    Ok(())
}

fn bump_state_rev(draft: &mut Draft) -> Result<(), Reason> {
    draft.state.state_rev = draft
        .state
        .state_rev
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    normalize(&mut draft.state);
    Ok(())
}

/// Seal a migrate record for `now`, which is already the journal tip or the on-disk save.
/// `edited` means `now` is a hand edit of the checkpoint head. The journal does not hold
/// that edit, so a checkpoint of the migrated save follows the record and replay restarts
/// from it. The caller holds the pack lock.
pub fn commit_migrate(
    root: &Path,
    now: State,
    edited: bool,
    lock: &PackGuard,
) -> Result<(), Reason> {
    if !root.join("pack.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    let canon = crate::canon::load_canon(root).map_err(|diags| first_reason(&diags))?;
    let pack_rev_after = pack_rev(root)?;
    let pack_rev_before = now.pack_rev.clone();
    let state_rev_before = now.state_rev;
    let state_rev_after = state_rev_before
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    let added = missing_meters(&canon, &now);
    let now = migrate_save(now, &pack_rev_after, state_rev_after, &added);
    check_save(&canon, &now).map_err(|diags| first_reason(&diags))?;
    let emitted = emit_save(&now)?.into_bytes();
    let entry = journal::migrate_record(
        &pack_rev_before,
        &now.pack_rev,
        state_rev_before,
        now.state_rev,
        &blake2s_tagged(&emitted),
        &added,
        edited,
    );
    let write = lock_write(root, lock)?;
    let prev = journal::read_chain_tip(root).map_err(|_| Reason::Schema)?;
    let line = journal::seal(&entry, &prev);
    if !edited {
        append_to_bundle(root, &line, &write)?;
        return Ok(());
    }
    // The snapshot lands before the record, so a crash after the append still replays.
    write_atomic(&journal::checkpoint_path(root, now.state_rev), &emitted)?;
    let (payload, chain) = seal_checkpoint(line, &now, &emitted)?;
    append_to_bundle(root, &payload, &write)?;
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
fn seal_checkpoint(
    line: String,
    now: &State,
    now_bytes: &[u8],
) -> Result<(String, String), Reason> {
    let prev = journal::sealed_chain(&line).map_err(|_| Reason::Schema)?;
    let mark =
        journal::checkpoint_record(now.round_index, now.state_rev, &blake2s_tagged(now_bytes));
    let sealed = journal::seal(&mark, &prev);
    let chain = journal::sealed_chain(&sealed).map_err(|_| Reason::Schema)?;
    Ok((format!("{line}{sealed}"), chain))
}

fn seal_round(
    line: PlayedRound,
    state_rev_before: u64,
    draft: &Draft,
    now_bytes: &[u8],
    root: &Path,
) -> Result<String, Reason> {
    let record = journal::RoundRecord {
        kind: journal::RoundKind::Round,
        round_index: line.clock.round_index,
        rules_rev: crate::store::RULES_REV,
        state_rev_before,
        state_rev_after: draft.state.state_rev,
        world_time_before: line.clock.world_time_before,
        world_time_after: line.clock.world_time_after,
        presence_players: line.presence.players,
        actors_present: line.presence.actors_present,
        seed: line.seed,
        last_round_at: line.last_round_at,
        intents: line.intents,
        refused: line.log.refused,
        effects: line.log.effects,
        arrivals: line.log.arrivals,
        fast_forward: line.fast_forward,
        dice: line.log.dice,
        extra_steps: line.log.extra_steps,
        memories: draft.memories.clone(),
        state_hash: blake2s_tagged(now_bytes),
        prev: String::new(),
        chain: String::new(),
    };
    let prev = journal::read_chain_tip(root).map_err(|_| Reason::Schema)?;
    Ok(journal::seal(&journal::Record::Round(record), &prev))
}

fn is_checkpoint_due(every: u64, round_index: u64) -> bool {
    every > 0 && round_index > 0 && round_index.is_multiple_of(every)
}

pub fn append_to_bundle(root: &Path, payload: &str, write: &WriteGuard) -> Result<String, Reason> {
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
    let head = serde_json::to_string(&journal::CheckpointHead {
        rules_rev: crate::store::RULES_REV,
        state_rev,
        round_index,
        state_hash: blake2s_tagged(now_bytes),
        chain: chain.to_string(),
    })
    .map_err(|_| Reason::Schema)?;
    write_atomic(
        &root.join("state").join(journal::head_path()),
        head.as_bytes(),
    )?;
    publish_memory_files(root, write)?;
    let state = root.join("state");
    let save = state.join("save.yaml");
    let base = state.join(journal::base_path());
    if !base.is_file() {
        fs::copy(&save, &base).map_err(|_| Reason::CommitConflict)?;
        fsync_file(&base)?;
        if let Some(parent) = base.parent() {
            fsync_dir(parent)?;
        }
    }
    let snap_dir = state.join("checkpoints");
    fs::create_dir_all(&snap_dir).map_err(|_| Reason::CommitConflict)?;
    write_atomic(&save, now_bytes)?;
    write_atomic(&snap_dir.join(format!("{state_rev}.yaml")), now_bytes)?;
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
pub fn finish_live_checkpoint<L>(world: &World<L>, lock: &PackGuard) -> Result<(), Reason> {
    finish_trailing_checkpoint(world, &lock_write(&world.root, lock)?)
}

fn finish_trailing_checkpoint<L>(world: &World<L>, write: &WriteGuard) -> Result<(), Reason> {
    let Some(path) = journal::find_live_bundle(&world.root) else {
        return Ok(());
    };
    let Some(mark) = journal::trailing_checkpoint(&path) else {
        return Ok(());
    };
    let now_bytes = emit_save(world.state())?.into_bytes();
    if blake2s_tagged(&now_bytes) != mark.state_hash {
        return Ok(());
    }
    let save = world.root.join("state/save.yaml");
    let snap = journal::checkpoint_path(&world.root, mark.state_rev);
    let save_ok = fs::read(&save)
        .ok()
        .is_some_and(|bytes| blake2s_tagged(&bytes) == mark.state_hash);
    let snap_ok = fs::read(&snap)
        .ok()
        .is_some_and(|bytes| blake2s_tagged(&bytes) == mark.state_hash);
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

fn publish_memory_files(root: &Path, _write: &WriteGuard) -> Result<(), Reason> {
    let entries = journal::read_live_records(root).map_err(|_| Reason::Schema)?;
    let mut lines = Vec::new();
    for entry in entries {
        if let journal::Record::Round(record) = entry {
            lines.extend(record.memories);
        }
    }
    for (actor, body) in memory_bodies(&root.join("state"), &lines)? {
        let path = root.join("state/memories").join(format!("{actor}.jsonl"));
        write_atomic(&path, body.as_bytes())?;
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
    use crate::round::log::{PlayedRound, RoundLog};
    use crate::round::stage::advance::RoundTime;
    use crate::store::durability::{self, Event};
    use crate::store::lock::lock_pack;
    use crate::testing::copy_docks;
    use crate::world::World;

    use super::commit;

    static NOBODY: Presence = Presence {
        players: 0,
        actors_present: Vec::new(),
    };

    #[test]
    fn a_reopen_keeps_every_journal_record_the_commit_wrote() {
        let root = copy_docks("journal-reopen");
        let mut world = crate::testing::resume(&root);
        let pre = fs::read(root.join("state/save.yaml")).unwrap();
        let mut draft = Draft::begin(&world, &NOBODY);
        draft.memories.push(MemoryLine {
            round_index: 0,
            actor_id: "mira".to_string(),
            intent_id: "rest".to_string(),
            fact_ids: vec![],
            tags: vec![],
            text: Some("kept".to_string()),
        });
        let line = round_line(0);
        durability::start_trace();
        commit(&mut world, draft, line).unwrap();
        let marks = durability::take_trace();
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
        let next = Draft::begin(&world, &NOBODY);
        durability::start_trace();
        commit(&mut world, next, round_line(1)).unwrap();
        let marks = durability::take_trace();
        assert_eq!(labels(&marks), ["file bundle"], "{}", show(&marks));
        let bundle_bytes = fs::read(&bundle).unwrap();
        drop(world);
        drop(lock_pack(&root).unwrap());
        let opened = World::open(&root).unwrap();
        assert_eq!(opened.state().state_rev, 0);
        assert_eq!(fs::read(&bundle).unwrap(), bundle_bytes);
        assert!(fs::read_to_string(&memory).unwrap().contains("kept"));
        assert!(!root.join("state/.txn").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_gzip_keeps_the_round_and_the_next_commit_starts_a_bundle() {
        let root = copy_docks("gzip-fail");
        let _clear = FailGuard;
        set_checkpoint_every(&root, 1);
        let mut world = crate::testing::resume(&root);
        let mut draft = Draft::begin(&world, &NOBODY);
        draft.state.round_index = 1;
        durability::fail_next(".jsonl.gz.tmp");
        commit(&mut world, draft, round_line(1)).unwrap();
        let live = crate::journal::read_live_records(&root).unwrap();
        let round_rev = live.iter().rev().find_map(|entry| match entry {
            crate::journal::Record::Round(record) => Some(record.state_rev_after),
            _ => None,
        });
        let mark = crate::journal::latest_checkpoint(&live).unwrap();
        assert_eq!(world.state().state_rev, round_rev.unwrap());
        assert_eq!(world.state().state_rev, mark.state_rev);
        assert_eq!(world.take_warning(), Some(super::CHECKPOINT_WARNING));
        assert!(root.join("state/journal/bundle-0000.jsonl").is_file());
        assert!(!root.join("state/journal/bundle-0000.jsonl.gz").is_file());

        let mut again = Draft::begin(&world, &NOBODY);
        again.state.round_index = 2;
        commit(&mut world, again, round_line(2)).unwrap();
        assert!(world.take_warning().is_none());
        let bundles = crate::journal::read_bundle_records(&root).unwrap();
        assert_eq!(bundles.len(), 2, "{bundles:?}");
        for (name, entries) in &bundles {
            let checkpoints = entries
                .iter()
                .filter(|entry| matches!(entry, crate::journal::Record::Checkpoint(_)))
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
        let mut first = Draft::begin(&world, &NOBODY);
        first.state.round_index = 1;
        commit(&mut world, first, round_line(1)).unwrap();

        let mut second = Draft::begin(&world, &NOBODY);
        second.state.round_index = 2;
        durability::fail_next("save.yaml.tmp");
        commit(&mut world, second, round_line(2)).unwrap();
        let live = crate::journal::read_live_records(&root).unwrap();
        let mark = crate::journal::latest_checkpoint(&live).unwrap();
        assert_eq!(world.state().state_rev, mark.state_rev);
        assert_eq!(world.take_warning(), Some(super::CHECKPOINT_WARNING));

        let mut third = Draft::begin(&world, &NOBODY);
        third.state.round_index = 3;
        commit(&mut world, third, round_line(3)).unwrap();
        let bundles = crate::journal::read_bundle_records(&root).unwrap();
        assert_eq!(bundles.len(), 2, "{bundles:?}");
        let (first_name, first_entries) = &bundles[0];
        let (second_name, second_entries) = &bundles[1];
        assert_eq!(count_checkpoints(first_entries), 1, "{first_name}");
        assert_eq!(count_checkpoints(second_entries), 0, "{second_name}");
        assert!(second_entries.iter().any(|entry| {
            matches!(entry, crate::journal::Record::Round(record) if record.round_index == 3)
        }));
        let saved = fs::read(root.join("state/save.yaml")).unwrap();
        let kept = crate::journal::latest_checkpoint(first_entries).unwrap();
        assert_eq!(crate::journal::blake2s_tagged(&saved), kept.state_hash);
        let snap = crate::journal::checkpoint_path(&root, kept.state_rev);
        assert_eq!(fs::read(snap).unwrap(), saved);
        let _ = fs::remove_dir_all(&root);
    }

    fn count_checkpoints(entries: &[crate::journal::Record]) -> usize {
        entries
            .iter()
            .filter(|entry| matches!(entry, crate::journal::Record::Checkpoint(_)))
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

    fn labels(marks: &[Event]) -> Vec<String> {
        marks.iter().map(label).collect()
    }

    fn label(mark: &Event) -> String {
        let path = match mark {
            Event::File(path) | Event::Dir(path) | Event::Rename(path) => path.as_path(),
        };
        let name = path
            .file_name()
            .and_then(|file| file.to_str())
            .unwrap_or("");
        match mark {
            Event::File(_) if name.ends_with(".tmp") => tmp_label(name, path),
            Event::File(_) if name == "COMPLETE" && has_parent_named(path, ".txn") => {
                "file complete".to_string()
            }
            Event::File(_) if name == "save.yaml" && has_parent_named(path, "prev") => {
                "file save".to_string()
            }
            Event::Dir(_) if name == "prev" && has_parent_named(path, ".txn") => {
                "dir prev".to_string()
            }
            Event::Dir(_) if name == ".txn" => "dir txn".to_string(),
            Event::File(_) if name.starts_with("bundle-") && has_parent_named(path, "journal") => {
                "file bundle".to_string()
            }
            Event::Dir(_) if name == "journal" && has_parent_named(path, "state") => {
                "dir journal".to_string()
            }
            Event::Dir(_) if name == "memories" && has_parent_named(path, "state") => {
                "dir memories".to_string()
            }
            Event::Dir(_) if name == "state" => "dir state".to_string(),
            Event::Rename(_) if name == "base.yaml" && has_parent_named(path, "journal") => {
                "rename base".to_string()
            }
            Event::Rename(_) if name.starts_with("tick-") => "rename tick".to_string(),
            Event::Rename(_) if has_parent_named(path, "memories") => "rename memory".to_string(),
            Event::Rename(_) if name == "save.yaml" && has_parent_named(path, "state") => {
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
        } else if has_parent_named(path, "memories") {
            "file memory.tmp".to_string()
        } else {
            format!("unexpected {}", path.display())
        }
    }

    fn has_parent_named(path: &Path, name: &str) -> bool {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|file| file.to_str())
            == Some(name)
    }

    fn show(marks: &[Event]) -> String {
        marks
            .iter()
            .map(|mark| match mark {
                Event::File(path) => format!("file {}", path.display()),
                Event::Dir(path) => format!("dir {}", path.display()),
                Event::Rename(path) => format!("rename {}", path.display()),
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

    fn round_line(round_index: u64) -> PlayedRound {
        PlayedRound {
            clock: RoundTime {
                round_index,
                world_time_before: 0,
                world_time_after: 0,
            },
            presence: Presence::default(),
            seed: String::new(),
            last_round_at: String::new(),
            fast_forward: false,
            log: RoundLog::default(),
            intents: Vec::new(),
        }
    }
}
