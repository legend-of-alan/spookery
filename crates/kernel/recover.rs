use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::codec::load_now;
use crate::diagnostic::reason_of;
use crate::durable::{fsync_dir, fsync_file, tmp_path};
use crate::error::Reason;

pub(crate) fn recover(root: &Path) -> Result<(), Reason> {
    let state = root.join("state");
    let txn = state.join(".txn");
    if txn.is_dir() {
        if !txn.join("COMPLETE").is_file() {
            fs::remove_dir_all(&txn).map_err(|_| Reason::CommitConflict)?;
        } else {
            let prev_now = txn.join("prev/now.yaml");
            if !prev_now.is_file() {
                return Err(Reason::Schema);
            }
            let prev = load_now(&fs::read_to_string(&prev_now).map_err(|_| Reason::Schema)?)
                .map_err(reason_of)?;
            let live = load_now(
                &fs::read_to_string(state.join("now.yaml")).map_err(|_| Reason::NotInSpec)?,
            )
            .map_err(reason_of)?;
            if live.state_rev == prev.state_rev.saturating_add(1)
                && live.state_rev != prev.state_rev
            {
                fs::remove_dir_all(&txn).map_err(|_| Reason::CommitConflict)?;
            } else if live.state_rev == prev.state_rev {
                restore_prev(&txn.join("prev"), &state)?;
                fs::remove_dir_all(&txn).map_err(|_| Reason::CommitConflict)?;
            } else {
                return Err(Reason::Schema);
            }
        }
    }

    let now = load_now(&fs::read_to_string(state.join("now.yaml")).map_err(|_| Reason::NotInSpec)?)
        .map_err(reason_of)?;
    let log_dir = state.join("log");
    let mut live_ticks = BTreeSet::new();
    if log_dir.is_dir() {
        let mut entries = fs::read_dir(&log_dir)
            .map_err(|_| Reason::Schema)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Reason::Schema)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("tick-") || !name.ends_with(".jsonl") {
                continue;
            }
            let text = fs::read_to_string(&path).map_err(|_| Reason::Schema)?;
            match (
                json_u64(&text, "tick_index"),
                json_u64(&text, "state_rev_after"),
            ) {
                (Some(tick_index), Some(rev)) if rev <= now.state_rev => {
                    live_ticks.insert(tick_index);
                }
                _ => {
                    fs::remove_file(&path).map_err(|_| Reason::CommitConflict)?;
                }
            }
        }
        drop_ahead(&log_dir, "presence-", ".json", now.state_rev)?;
        drop_ahead(&log_dir, "migrate-", ".json", now.state_rev)?;
        drop_future_base(&log_dir, now.state_rev)?;
    }

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
                match json_u64(line, "tick_index") {
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

fn restore_prev(prev: &Path, state: &Path) -> Result<(), Reason> {
    let mut files = Vec::new();
    collect_rel_files(prev, prev, &mut files)?;
    for rel in files {
        let from = prev.join(&rel);
        let to = state.join(&rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|_| Reason::CommitConflict)?;
        }
        fs::copy(&from, &to).map_err(|_| Reason::CommitConflict)?;
        fsync_file(&to)?;
    }
    fsync_dir(state)?;
    Ok(())
}

fn collect_rel_files(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) -> Result<(), Reason> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(dir)
        .map_err(|_| Reason::Schema)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Reason::Schema)?;
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_rel_files(&path, root, out)?;
        } else {
            out.push(
                path.strip_prefix(root)
                    .map_err(|_| Reason::Schema)?
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

fn drop_ahead(log_dir: &Path, prefix: &str, suffix: &str, live_rev: u64) -> Result<(), Reason> {
    let mut entries = fs::read_dir(log_dir)
        .map_err(|_| Reason::Schema)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Reason::Schema)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(prefix) || !name.ends_with(suffix) {
            continue;
        }
        let path = entry.path();
        let text = fs::read_to_string(&path).map_err(|_| Reason::Schema)?;
        match json_u64(&text, "state_rev_after") {
            Some(rev) if rev <= live_rev => {}
            _ => {
                fs::remove_file(&path).map_err(|_| Reason::CommitConflict)?;
            }
        }
    }
    Ok(())
}

fn drop_future_base(log_dir: &Path, live_rev: u64) -> Result<(), Reason> {
    let path = log_dir.join("base.yaml");
    if !path.is_file() {
        return Ok(());
    }
    let text = fs::read_to_string(&path).map_err(|_| Reason::Schema)?;
    let saved = load_now(&text).map_err(reason_of)?;
    if saved.state_rev > live_rev {
        fs::remove_file(&path).map_err(|_| Reason::CommitConflict)?;
    }
    Ok(())
}

fn json_u64(text: &str, key: &str) -> Option<u64> {
    let pattern = format!("\"{key}\":");
    let index = text.find(&pattern)?;
    let rest = text[index + pattern.len()..].trim_start();
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
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

    use crate::codec::{emit_now, load_now};
    use crate::commit::commit_presence;
    use crate::journal::hash_now;
    use crate::model::Presence;
    use crate::World;

    #[test]
    fn a_presence_commit_journals_the_new_presence_and_the_base() {
        let root = temp_docks("presence-journal");
        let mut world = World::open(&root).unwrap();
        let pre = fs::read(root.join("state/now.yaml")).unwrap();
        let changed = commit_presence(
            &mut world,
            Presence {
                humans: 1,
                actors_present: vec!["pc_rowan".to_string()],
            },
        )
        .unwrap();
        assert!(changed);
        assert_eq!(fs::read(root.join("state/log/base.yaml")).unwrap(), pre);
        let now = fs::read(root.join("state/now.yaml")).unwrap();
        let record = fs::read_to_string(root.join("state/log/presence-0001.json")).unwrap();
        assert!(record.contains(&hash_now(&now)), "{record}");
        assert!(record.contains("\"state_rev_after\":1"), "{record}");
        assert_eq!(world.now().state_rev, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_crash_between_the_journal_rename_and_now_rolls_back() {
        let root = temp_docks("crash-window");
        let mut world = World::open(&root).unwrap();
        let pre = fs::read(root.join("state/now.yaml")).unwrap();
        commit_presence(
            &mut world,
            Presence {
                humans: 1,
                actors_present: vec!["pc_rowan".to_string()],
            },
        )
        .unwrap();
        drop(world);
        fs::write(root.join("state/now.yaml"), &pre).unwrap();
        let txn = root.join("state/.txn");
        fs::create_dir_all(txn.join("prev")).unwrap();
        fs::write(txn.join("prev/now.yaml"), &pre).unwrap();
        fs::write(txn.join("COMPLETE"), b"").unwrap();
        let opened = World::open(&root).unwrap();
        assert_eq!(opened.now().state_rev, 0);
        assert_eq!(fs::read(root.join("state/now.yaml")).unwrap(), pre);
        assert!(!root.join("state/log/presence-0001.json").exists());
        assert_eq!(fs::read(root.join("state/log/base.yaml")).unwrap(), pre);
        assert!(!txn.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_base_ahead_of_the_save_is_dropped_with_its_record() {
        let root = temp_docks("base-ahead");
        let now_path = root.join("state/now.yaml");
        let pre = fs::read(&now_path).unwrap();
        let mut ahead = load_now(std::str::from_utf8(&pre).unwrap()).unwrap();
        ahead.state_rev = 1;
        let ahead_bytes = emit_now(&ahead).unwrap().into_bytes();
        let log = root.join("state/log");
        fs::create_dir_all(&log).unwrap();
        fs::write(log.join("base.yaml"), &ahead_bytes).unwrap();
        fs::write(
            log.join("migrate-0001.json"),
            "{\"kind\":\"migrate\",\"state_rev_after\":1}\n",
        )
        .unwrap();
        let txn = root.join("state/.txn");
        fs::create_dir_all(txn.join("prev")).unwrap();
        fs::write(txn.join("prev/now.yaml"), &pre).unwrap();
        fs::write(txn.join("COMPLETE"), b"").unwrap();
        let opened = World::open(&root).unwrap();
        assert_eq!(opened.now().state_rev, 0);
        assert_eq!(fs::read(&now_path).unwrap(), pre);
        assert!(!log.join("base.yaml").exists());
        assert!(!log.join("migrate-0001.json").exists());
        let _ = fs::remove_dir_all(&root);
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
