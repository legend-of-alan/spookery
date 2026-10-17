use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::error::Reason;
use crate::journal::{self, JournalEntry};
use crate::store::durable::{fsync_dir, fsync_file, tmp_path};

pub(crate) fn recover(root: &Path) -> Result<(), Reason> {
    let state = root.join("state");
    let txn = state.join(".txn");
    if txn.is_dir() {
        fs::remove_dir_all(&txn).map_err(|_| Reason::CommitConflict)?;
    }
    cut_torn_tail(root)?;

    let records = journal::read_entries(root).map_err(|fault| {
        if fault.detail.contains("RULES_REV") {
            Reason::RulesRev
        } else {
            Reason::Schema
        }
    })?;
    let live_ticks: BTreeSet<u64> = records
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Tick(record) => Some(record.tick_index),
            _ => None,
        })
        .collect();

    let memories = state.join("memories");
    if memories.is_dir() {
        let entries = fs::read_dir(&memories)
            .map_err(|_| Reason::Schema)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Reason::Schema)?;
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let text = fs::read_to_string(&path).map_err(|_| Reason::Schema)?;
            let mut kept = String::new();
            let mut changed = false;
            for line in text.lines() {
                if line.is_empty() {
                    continue;
                }
                match memory_tick(line) {
                    Some(tick_index) if live_ticks.contains(&tick_index) => {
                        kept.push_str(line);
                        kept.push('\n');
                    }
                    _ => changed = true,
                }
            }
            if changed {
                atomic_write(&path, kept.as_bytes())?;
            }
        }
    }

    delete_tmp(&state);
    Ok(())
}

fn cut_torn_tail(root: &Path) -> Result<(), Reason> {
    let Some(path) = journal::live_plain(root) else {
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
            tool_trace: Vec::new(),
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
