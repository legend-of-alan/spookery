//! Finish a commit a crash left half done, or roll it back.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::Reason;
use crate::journal::{self, JournalEntry};
use crate::model::MemoryLine;
use crate::store::codec::emit_memory_line;
use crate::store::durable::{fsync_dir, fsync_file, rename, tmp_path};

pub(crate) fn recover(root: &Path) -> Result<(), Reason> {
    let state = root.join("state");
    let txn = state.join(".txn");
    if txn.is_dir() {
        fs::remove_dir_all(&txn).map_err(|_| Reason::CommitConflict)?;
    }
    cut_torn_tail(root)?;
    rebuild_memory_files(root)?;
    close_finished(root)?;
    delete_tmp(&state);
    Ok(())
}

/// Gzip every plain bundle that ends in a published checkpoint.
/// When the gzip is already complete, only the plain file is removed.
pub(crate) fn close_finished(root: &Path) -> Result<(), Reason> {
    let mut failed = Ok(());
    for plain in plain_bundles(root) {
        if journal::trailing_checkpoint(&plain).is_none() {
            continue;
        }
        if let Err(reason) = close_one(root, &plain) {
            failed = Err(reason);
        }
    }
    failed
}

fn close_one(root: &Path, plain: &Path) -> Result<(), Reason> {
    let bytes = fs::read(plain).map_err(|_| Reason::CommitConflict)?;
    let gz = gzip_path(plain);
    if gz_matches(&gz, &bytes) {
        ensure_snapshot(root, plain)?;
        return remove_plain(plain);
    }
    if !journal::published_checkpoint(root, plain) {
        return Ok(());
    }
    ensure_snapshot(root, plain)?;
    let tmp = tmp_path(&gz);
    let gz_bytes = journal::gzip_bytes(&bytes).map_err(|_| Reason::CommitConflict)?;
    fs::write(&tmp, gz_bytes).map_err(|_| Reason::CommitConflict)?;
    fsync_file(&tmp)?;
    rename(&tmp, &gz)?;
    remove_plain(plain)
}

fn ensure_snapshot(root: &Path, plain: &Path) -> Result<(), Reason> {
    let Some(mark) = journal::trailing_checkpoint(plain) else {
        return Ok(());
    };
    let save = fs::read(root.join("state/save.yaml")).map_err(|_| Reason::CommitConflict)?;
    if journal::hash_now(&save) != mark.state_hash {
        return Ok(());
    }
    let snap = root
        .join("state/checkpoints")
        .join(format!("{}.yaml", mark.state_rev));
    if fs::read(&snap).ok().as_deref() == Some(save.as_slice()) {
        return Ok(());
    }
    atomic_write(&snap, &save)
}

fn gz_matches(gz: &Path, plain: &[u8]) -> bool {
    if !gz.is_file() {
        return false;
    }
    let Ok(bytes) = fs::read(gz) else {
        return false;
    };
    journal::gunzip_bytes(&bytes).ok().as_deref() == Some(plain)
}

fn remove_plain(plain: &Path) -> Result<(), Reason> {
    fs::remove_file(plain).map_err(|_| Reason::CommitConflict)?;
    if let Some(parent) = plain.parent() {
        fsync_dir(parent)?;
    }
    Ok(())
}

fn plain_bundles(root: &Path) -> Vec<PathBuf> {
    let dir = root.join("state/journal");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut plains = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with("bundle-") && name.ends_with(".jsonl") {
            plains.push(entry.path());
        }
    }
    plains.sort();
    plains
}

fn gzip_path(plain: &Path) -> PathBuf {
    let mut name = plain.file_name().unwrap_or_default().to_os_string();
    name.push(".gz");
    plain.with_file_name(name)
}

fn reason_for(detail: &str) -> Reason {
    if detail.contains("RULES_REV") {
        Reason::RulesRev
    } else {
        Reason::Schema
    }
}

/// Memory files are a cache of the journal. The live bundle supplies lines since
/// the last checkpoint. A line for a tick the live bundle does not have, and
/// that the checkpoint does not cover, is dropped.
fn rebuild_memory_files(root: &Path) -> Result<(), Reason> {
    let records = journal::read_live(root).map_err(|fault| reason_for(&fault.detail))?;
    let head = journal::read_head(root).map_err(|fault| reason_for(&fault.detail))?;
    let live_ticks: BTreeSet<u64> = records
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Tick(record) => Some(record.tick_index),
            _ => None,
        })
        .collect();
    let fresh: Vec<MemoryLine> = records
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Tick(record) => Some(record.memories.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let memories = root.join("state/memories");
    let mut actors = BTreeSet::new();
    if memories.is_dir() {
        for entry in fs::read_dir(&memories)
            .map_err(|_| Reason::Schema)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Reason::Schema)?
        {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                if let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) {
                    actors.insert(name.to_string());
                }
            }
        }
    }
    for line in &fresh {
        actors.insert(line.actor_id.clone());
    }
    if actors.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(&memories).map_err(|_| Reason::Schema)?;
    for actor in actors {
        let path = memories.join(format!("{actor}.jsonl"));
        let mut kept = Vec::new();
        if path.is_file() {
            let text = fs::read_to_string(&path).map_err(|_| Reason::Schema)?;
            for line in text.lines() {
                if line.is_empty() {
                    continue;
                }
                if memory_tick(line).is_some_and(|tick| keep_tick(tick, head.as_ref(), &live_ticks))
                {
                    kept.push(line.to_string());
                }
            }
        }
        for line in fresh.iter().filter(|line| line.actor_id == actor) {
            let text = emit_memory_line(line);
            if !kept.iter().any(|row| row == &text) {
                kept.push(text);
            }
        }
        let mut body = kept.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        let current = fs::read_to_string(&path).unwrap_or_default();
        if path.is_file() && current == body {
            continue;
        }
        if body.is_empty() && !path.is_file() {
            continue;
        }
        atomic_write(&path, body.as_bytes())?;
    }
    Ok(())
}

fn keep_tick(tick: u64, head: Option<&journal::BundleHead>, live_ticks: &BTreeSet<u64>) -> bool {
    if live_ticks.contains(&tick) {
        return true;
    }
    match head {
        Some(head) => tick <= head.tick_index,
        None => false,
    }
}

fn cut_torn_tail(root: &Path) -> Result<(), Reason> {
    let Some(path) = journal::find_live_bundle(root) else {
        return Ok(());
    };
    let bytes = fs::read(&path).map_err(|_| Reason::Schema)?;
    if bytes.is_empty() || bytes.ends_with(b"\n") {
        return Ok(());
    }
    let end = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|at| at + 1)
        .unwrap_or(0);
    let file = OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(|_| Reason::CommitConflict)?;
    file.set_len(end as u64)
        .map_err(|_| Reason::CommitConflict)?;
    fsync_file(&path)?;
    Ok(())
}

fn memory_tick(line: &str) -> Option<u64> {
    #[derive(serde::Deserialize)]
    struct MemoryTick {
        tick_index: u64,
    }
    serde_json::from_str::<MemoryTick>(line)
        .ok()
        .map(|row| row.tick_index)
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), Reason> {
    let tmp = tmp_path(target);
    if let Some(parent) = tmp.parent() {
        fs::create_dir_all(parent).map_err(|_| Reason::CommitConflict)?;
    }
    {
        let mut file = File::create(&tmp).map_err(|_| Reason::CommitConflict)?;
        file.write_all(bytes).map_err(|_| Reason::CommitConflict)?;
        file.sync_all().map_err(|_| Reason::CommitConflict)?;
    }
    fs::rename(&tmp, target).map_err(|_| Reason::CommitConflict)?;
    if let Some(parent) = target.parent() {
        fsync_dir(parent)?;
    }
    Ok(())
}

fn delete_tmp(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name() == ".txn" {
                continue;
            }
            delete_tmp(&path);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".tmp"))
        {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::commit::commit_round;
    use crate::draft::Draft;
    use crate::world::World;

    #[test]
    fn a_torn_tail_is_cut_off_and_the_save_stays() {
        let root = temp_docks("torn-tail");
        let mut world = World::open(&root).unwrap();
        let pre = fs::read(root.join("state/save.yaml")).unwrap();
        let draft = Draft::begin(&world);
        commit_round(&mut world, draft, &journal_line()).unwrap();
        drop(world);
        let bundle = root.join("state/journal/bundle-0000.jsonl");
        let kept = fs::read(&bundle).unwrap();
        let mut torn = kept.clone();
        torn.extend(br#"{"tick_index":99,"#);
        fs::write(&bundle, &torn).unwrap();
        let opened = World::open(&root).unwrap();
        assert_eq!(opened.now().state_rev, 0);
        assert_eq!(fs::read(root.join("state/save.yaml")).unwrap(), pre);
        assert_eq!(fs::read(&bundle).unwrap(), kept);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_old_one_file_record_is_refused() {
        let root = temp_docks("old-layout");
        let log = root.join("state/journal");
        fs::create_dir_all(&log).unwrap();
        fs::write(log.join("migrate-0001.json"), "{}\n").unwrap();
        match World::open(&root) {
            Ok(_) => panic!("old layout opened"),
            Err(err) => assert_eq!(err, crate::error::Reason::RulesRev),
        }
        let _ = fs::remove_dir_all(&root);
    }

    fn journal_line() -> String {
        use crate::journal::{emit_entry, JournalEntry, TickRecord};
        emit_entry(&JournalEntry::Tick(TickRecord {
            tick_index: 0,
            rules_rev: 7,
            state_rev_before: 0,
            state_rev_after: 1,
            world_time_before: 0,
            world_time_after: 0,
            presence_humans: 0,
            actors_present: Vec::new(),
            seed: String::new(),
            last_tick_at: String::new(),
            intents: Vec::new(),
            rejected: Vec::new(),
            effects: Vec::new(),
            arrivals: Vec::new(),
            fast_forward: false,
            dice: Vec::new(),
            steps_run: Vec::new(),
            memories: Vec::new(),
            now_hash: String::new(),
            prev: String::new(),
            chain: String::new(),
        }))
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
        copy_dir(&source, &root);
        root
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &dest);
            } else {
                fs::copy(entry.path(), dest).unwrap();
            }
        }
    }
}
