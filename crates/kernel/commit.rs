use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::canon::pack_rev;
use crate::error::Reason;
use crate::journal::{self, hash_now};
use crate::lock::lock_pack;
use crate::model::{MemoryLine, Now};
use crate::store::codec::{canonicalize, emit_memory_line, emit_save, load_save};
use crate::store::durable::{fsync_dir, fsync_dirs, fsync_file, rename, tmp_path};
use crate::world::{Draft, World};

pub struct CommitOutcome {
    pub state_rev: u64,
    pub diff_summary: Vec<String>,
    pub log_path: String,
}

pub fn commit_round(world: &mut World, mut draft: Draft, round_jsonl: &str) -> Result<(), Reason> {
    draft.state.state_rev = draft
        .state
        .state_rev
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    canonicalize(&mut draft.state);
    let now_bytes = emit_save(&draft.state)?.into_bytes();
    let stamped = journal::stamp_now_hash(round_jsonl, &now_bytes)?;
    let tick_index = draft.state.tick_index;
    let state = world.root.join("state");
    let pre = read_pre_now(&state)?;
    let mut planned = plan_commit(
        &state,
        now_bytes.clone(),
        Some((tick_index, stamped.as_bytes())),
        &draft.memories,
    )?;
    push_base(&mut planned, &state, &pre, &now_bytes, false);
    write_commit(&state, &planned)?;
    world.now = draft.state;
    Ok(())
}

pub fn commit_pending(world: &mut World) -> Result<CommitOutcome, Reason> {
    let pending = world.pending.take().ok_or(Reason::CommitConflict)?;
    let mut now = pending.now;
    now.state_rev = now.state_rev.checked_add(1).ok_or(Reason::CommitConflict)?;
    canonicalize(&mut now);
    let now_bytes = emit_save(&now)?.into_bytes();
    let stamped = journal::stamp_now_hash(&pending.round_jsonl, &now_bytes)?;
    let tick_index = pending.tick_index;
    let log_path = format!("state/journal/tick-{tick_index:04}.jsonl");
    let state = world.root.join("state");
    let pre = read_pre_now(&state)?;
    let mut planned = plan_commit(
        &state,
        now_bytes.clone(),
        Some((tick_index, stamped.as_bytes())),
        &pending.memories,
    )?;
    push_base(&mut planned, &state, &pre, &now_bytes, false);
    write_commit(&state, &planned)?;
    world.now = now;
    world.queue.clear();
    Ok(CommitOutcome {
        state_rev: world.now.state_rev,
        diff_summary: pending.paths,
        log_path,
    })
}

pub fn migrate_pack(root: &Path) -> Result<(), Reason> {
    if !root.join("pack.yaml").is_file() || !root.join("state/save.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    let now_bytes = fs::read(root.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
    let mut now = load_save(now_text).map_err(crate::diagnostic::reason_of)?;
    let canon = crate::canon::load_canon(root).map_err(crate::diagnostic::reason_of)?;
    crate::canon::bind_meters(&canon, &mut now).map_err(crate::diagnostic::reason_of)?;
    crate::canon::bind_inventory(&now).map_err(crate::diagnostic::reason_of)?;
    let unknown = crate::canon::unknown_save_items(&canon, &now);
    if !unknown.is_empty() {
        return Err(crate::diagnostic::reason_of(unknown));
    }
    let pack_rev_after = pack_rev(root)?;
    let pack_rev_before = now.pack_rev.clone();
    let state_rev_before = now.state_rev;
    let _lock = lock_pack(root)?;
    now.pack_rev = pack_rev_after;
    now.state_rev = now.state_rev.checked_add(1).ok_or(Reason::CommitConflict)?;
    canonicalize(&mut now);
    let emitted = emit_save(&now)?.into_bytes();
    let state = root.join("state");
    let record = journal::emit_migrate(
        &pack_rev_before,
        &now.pack_rev,
        state_rev_before,
        now.state_rev,
        &hash_now(&emitted),
    );
    let mut planned = plan_commit(&state, emitted.clone(), None, &[])?;
    push_base(&mut planned, &state, &now_bytes, &emitted, true);
    planned.push(Planned {
        target: state
            .join("journal")
            .join(journal::migrate_file(now.state_rev)),
        bytes: record.as_bytes().to_vec(),
        is_now: false,
    });
    write_commit(&state, &planned)?;
    append_studio(&state, &record)?;
    Ok(())
}

fn append_studio(state: &Path, line: &str) -> Result<(), Reason> {
    let log_dir = state.join("journal");
    fs::create_dir_all(&log_dir).map_err(|_| Reason::CommitConflict)?;
    let path = log_dir.join("studio.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|_| Reason::CommitConflict)?;
    file.write_all(line.as_bytes())
        .map_err(|_| Reason::CommitConflict)?;
    file.sync_all().map_err(|_| Reason::CommitConflict)?;
    fsync_dir(&log_dir)?;
    Ok(())
}

/// Replay writes `save.yaml` on a temp pack and does not append a journal record.
pub fn replay_write_now(world: &mut World, now: Now) -> Result<(), Reason> {
    let now_bytes = emit_save(&now)?.into_bytes();
    let state = world.root.join("state");
    let planned = vec![Planned {
        target: state.join("save.yaml"),
        bytes: now_bytes,
        is_now: true,
    }];
    write_commit(&state, &planned)?;
    world.now = now;
    Ok(())
}

fn read_pre_now(state: &Path) -> Result<Vec<u8>, Reason> {
    fs::read(state.join("save.yaml")).map_err(|_| Reason::NotInSpec)
}

/// The first commit stores the pre-commit save. Migrate stores the post-migrate save.
fn push_base(planned: &mut Vec<Planned>, state: &Path, pre: &[u8], post: &[u8], migrate: bool) {
    let target = state.join(journal::base_rel());
    let bytes = if migrate {
        post.to_vec()
    } else if target.is_file() {
        return;
    } else {
        pre.to_vec()
    };
    planned.push(Planned {
        target,
        bytes,
        is_now: false,
    });
}

fn plan_commit(
    state: &Path,
    now_bytes: Vec<u8>,
    tick: Option<(u64, &[u8])>,
    memories: &[MemoryLine],
) -> Result<Vec<Planned>, Reason> {
    let mut planned = Vec::new();
    planned.push(Planned {
        target: state.join("save.yaml"),
        bytes: now_bytes,
        is_now: true,
    });
    if let Some((tick_index, tick_jsonl)) = tick {
        planned.push(Planned {
            target: state.join(format!("journal/tick-{tick_index:04}.jsonl")),
            bytes: tick_jsonl.to_vec(),
            is_now: false,
        });
    }
    for (actor_id, body) in memory_bodies(state, memories)? {
        planned.push(Planned {
            target: state.join("memories").join(format!("{actor_id}.jsonl")),
            bytes: body.into_bytes(),
            is_now: false,
        });
    }
    Ok(planned)
}

struct Planned {
    target: PathBuf,
    bytes: Vec<u8>,
    is_now: bool,
}

fn write_commit(state: &Path, planned: &[Planned]) -> Result<(), Reason> {
    let txn_dir = state.join(".txn");
    let prev = txn_dir.join("prev");
    fs::create_dir_all(&prev).map_err(|_| Reason::CommitConflict)?;
    let mut backup_dirs = vec![prev.clone(), txn_dir.clone()];
    for item in planned {
        if !item.target.is_file() {
            continue;
        }
        let rel = item
            .target
            .strip_prefix(state)
            .map_err(|_| Reason::CommitConflict)?;
        let dest = prev.join(rel);
        if let Some(parent) = dest.parent() {
            if !parent.is_dir() {
                fs::create_dir_all(parent).map_err(|_| Reason::CommitConflict)?;
            }
            backup_dirs.push(parent.to_path_buf());
        }
        fs::copy(&item.target, &dest).map_err(|_| Reason::CommitConflict)?;
        fsync_file(&dest)?;
    }
    let complete = txn_dir.join("COMPLETE");
    File::create(&complete).map_err(|_| Reason::CommitConflict)?;
    fsync_file(&complete)?;
    fsync_dirs(&backup_dirs)?;

    let mut temps = Vec::new();
    let mut created_parents = Vec::new();
    for item in planned {
        let tmp = tmp_path(&item.target);
        if let Some(parent) = item.target.parent() {
            if !parent.is_dir() {
                fs::create_dir_all(parent).map_err(|_| Reason::CommitConflict)?;
                if let Some(dir_parent) = parent.parent() {
                    created_parents.push(dir_parent.to_path_buf());
                }
            }
        }
        {
            let mut file = File::create(&tmp).map_err(|_| Reason::CommitConflict)?;
            file.write_all(&item.bytes)
                .map_err(|_| Reason::CommitConflict)?;
        }
        fsync_file(&tmp)?;
        temps.push((tmp, item));
    }
    // The new directory is durable only after its parent is flushed.
    fsync_dirs(&created_parents)?;

    let mut side_parents = Vec::new();
    for (tmp, item) in &temps {
        if item.is_now {
            continue;
        }
        rename(tmp, &item.target)?;
        if let Some(parent) = item.target.parent() {
            if parent != state {
                side_parents.push(parent.to_path_buf());
            }
        }
    }
    fsync_dirs(&side_parents)?;
    for (tmp, item) in &temps {
        if !item.is_now {
            continue;
        }
        rename(tmp, &item.target)?;
    }
    fsync_dirs(&[state.to_path_buf()])?;
    let _ = fs::remove_dir_all(&txn_dir);
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
        for line in fresh {
            body.push_str(&emit_memory_line(line));
            body.push('\n');
        }
        out.insert(actor, body);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::model::MemoryLine;
    use crate::store::durable::{self, Mark};
    use crate::world::{Draft, World};

    use super::commit_round;

    #[test]
    fn a_reopen_keeps_every_journal_record_the_commit_wrote() {
        let root = temp_docks("journal-reopen");
        let mut world = World::open(&root).unwrap();
        let pre = fs::read(root.join("state/save.yaml")).unwrap();
        let mut draft = Draft::begin(&world);
        draft.memories.push(MemoryLine {
            tick_index: 0,
            actor_id: "mira".to_string(),
            intent_id: "rest".to_string(),
            fact_ids: vec![],
            tags: vec![],
            text: Some("kept".to_string()),
        });
        durable::start_trace();
        commit_round(
            &mut world,
            draft,
            r#"{"tick_index":0,"state_rev_after":1,"rules_rev":7}"#,
        )
        .unwrap();
        let marks = durable::take_trace();
        // Journal and the memory directory are flushed before save.yaml is renamed.
        assert_eq!(
            labels(&marks),
            [
                "file save",
                "file complete",
                "dir prev",
                "dir txn",
                "file save.tmp",
                "file tick.tmp",
                "file memory.tmp",
                "file base.tmp",
                "dir state",
                "rename tick",
                "rename memory",
                "rename base",
                "dir journal",
                "dir memories",
                "rename save",
                "dir state",
            ],
            "{}",
            show(&marks)
        );
        assert!(!root.join("state/.txn").exists());
        assert_no_tmp(&root);
        let tick = root.join("state/journal/tick-0000.jsonl");
        let base = root.join("state/journal/base.yaml");
        let memory = root.join("state/memories/mira.jsonl");
        let tick_bytes = fs::read(&tick).unwrap();
        let base_bytes = fs::read(&base).unwrap();
        let memory_bytes = fs::read(&memory).unwrap();
        assert_eq!(base_bytes, pre);
        let tick_text = String::from_utf8(tick_bytes.clone()).unwrap();
        assert!(tick_text.contains("\"state_rev_after\":1"), "{tick_text}");
        assert!(tick_text.contains("now_hash"), "{tick_text}");
        assert!(String::from_utf8(memory_bytes.clone())
            .unwrap()
            .contains("kept"));
        drop(world);
        let opened = World::open(&root).unwrap();
        assert_eq!(opened.now().state_rev, 1);
        assert_eq!(fs::read(&tick).unwrap(), tick_bytes);
        assert_eq!(fs::read(&base).unwrap(), base_bytes);
        assert_eq!(fs::read(&memory).unwrap(), memory_bytes);
        assert!(!root.join("state/.txn").exists());
        let _ = fs::remove_dir_all(&root);
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
