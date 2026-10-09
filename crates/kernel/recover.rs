use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::codec::{emit_rumors, load_now};
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
            let prev = load_now(&fs::read_to_string(&prev_now).map_err(|_| Reason::Schema)?)?;
            let live = load_now(
                &fs::read_to_string(state.join("now.yaml")).map_err(|_| Reason::NotInSpec)?,
            )?;
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

    let now =
        load_now(&fs::read_to_string(state.join("now.yaml")).map_err(|_| Reason::NotInSpec)?)?;
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

    let rumors_path = state.join("rumors.yaml");
    if rumors_path.is_file() {
        let text = fs::read_to_string(&rumors_path).map_err(|_| Reason::Schema)?;
        let parsed: Result<Vec<crate::model::Rumor>, _> = serde_saphyr::from_str(&text);
        let differs = match parsed {
            Ok(mut loaded) => {
                loaded.sort_by(|a, b| a.id.cmp(&b.id));
                let mut live = now.rumors.clone();
                live.sort_by(|a, b| a.id.cmp(&b.id));
                loaded != live
            }
            Err(_) => true,
        };
        if differs {
            atomic_write(&rumors_path, emit_rumors(&now)?.as_bytes())?;
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
