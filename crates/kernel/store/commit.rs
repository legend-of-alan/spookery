use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::canon::pack_rev;
use crate::draft::Draft;
use crate::error::Reason;
use crate::journal::{self, hash_now};
use crate::lock::lock_world;
use crate::model::{MemoryLine, Now};
use crate::rules::RollReport;
use crate::store::codec::{canonicalize, emit_memory_line, emit_save, load_save};
use crate::store::durable::{fsync_dir, fsync_file, rename, tmp_path};
use crate::world::{Delivered, World};

pub struct CommitOutcome {
    pub state_rev: u64,
    pub diff_summary: Vec<String>,
    pub log_path: String,
}

/// Where a played round's state goes.
pub enum RoundStore {
    /// The caller will write the round later.
    Hold,
    /// Append the journal record.
    Write,
    /// Keep the new state in memory. The journal already has this round.
    Memory,
}

/// Hold one played round when the caller will write it. Spook queues intents and takes rolls; it does not write this.
pub fn finish_round(
    world: &mut World,
    mut draft: Draft,
    store: RoundStore,
    record: String,
    notices: Vec<RollReport>,
    keep_queue: bool,
    delivered: Delivered,
) -> Result<(), Reason> {
    let humans = draft.now().presence.humans;
    let paths = world.store_delivery(delivered, humans);
    match store {
        RoundStore::Hold => {
            let tick_index = draft.now().tick_index;
            let now = draft.now().clone();
            let memories = draft.memories().to_vec();
            world.hold_uncommitted(now, record, tick_index, memories, paths, notices);
            Ok(())
        }
        RoundStore::Write => {
            commit_round(world, draft, &record)?;
            world.note_committed(notices, keep_queue);
            Ok(())
        }
        RoundStore::Memory => {
            draft.state.state_rev = draft
                .state
                .state_rev
                .checked_add(1)
                .ok_or(Reason::CommitConflict)?;
            canonicalize(&mut draft.state);
            world.now = draft.state;
            world.note_committed(notices, keep_queue);
            Ok(())
        }
    }
}

pub fn commit_round(world: &mut World, mut draft: Draft, round_jsonl: &str) -> Result<(), Reason> {
    draft.state.state_rev = draft
        .state
        .state_rev
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    canonicalize(&mut draft.state);
    let now_bytes = emit_save(&draft.state)?.into_bytes();
    let line = seal_tick(round_jsonl, &now_bytes, &world.root)?;
    let due = checkpoint_due(world.canon().checkpoint_every, draft.state.tick_index);
    let payload = if due {
        let prev = journal::sealed_chain(&line).map_err(|_| Reason::Schema)?;
        let mark = journal::checkpoint_entry(
            draft.state.tick_index,
            draft.state.state_rev,
            &hash_now(&now_bytes),
        );
        format!("{line}{}", journal::seal(&mark, &prev))
    } else {
        line
    };
    append_bundle(&world.root, &payload)?;
    write_memories(&world.root.join("state"), &draft.memories)?;
    if due {
        store_checkpoint(&world.root, &now_bytes, draft.state.state_rev)?;
    }
    world.now = draft.state;
    Ok(())
}

pub fn commit_pending(world: &mut World) -> Result<CommitOutcome, Reason> {
    let pending = world.pending.take().ok_or(Reason::CommitConflict)?;
    let mut now = pending.now;
    now.state_rev = now.state_rev.checked_add(1).ok_or(Reason::CommitConflict)?;
    canonicalize(&mut now);
    let now_bytes = emit_save(&now)?.into_bytes();
    let line = seal_tick(&pending.round_jsonl, &now_bytes, &world.root)?;
    let due = checkpoint_due(world.canon().checkpoint_every, now.tick_index);
    let payload = if due {
        let prev = journal::sealed_chain(&line).map_err(|_| Reason::Schema)?;
        let mark = journal::checkpoint_entry(now.tick_index, now.state_rev, &hash_now(&now_bytes));
        format!("{line}{}", journal::seal(&mark, &prev))
    } else {
        line
    };
    let mut log_path = append_bundle(&world.root, &payload)?;
    write_memories(&world.root.join("state"), &pending.memories)?;
    if due {
        store_checkpoint(&world.root, &now_bytes, now.state_rev)?;
        if !log_path.ends_with(".gz") {
            log_path.push_str(".gz");
        }
    }
    world.now = now;
    world.queue.clear();
    Ok(CommitOutcome {
        state_rev: world.now.state_rev,
        diff_summary: pending.paths,
        log_path,
    })
}

pub fn migrate_pack(root: &Path) -> Result<(), Reason> {
    let now = load_file_now(root)?;
    if journal_ahead(root, now.state_rev) {
        return Err(Reason::CommitConflict);
    }
    migrate_now(root, now)
}

/// Seal a migrate record for `now`, which is already the journal tip or the on-disk save.
pub fn migrate_now(root: &Path, mut now: Now) -> Result<(), Reason> {
    if !root.join("pack.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
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
    let _lock = lock_world(root)?;
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
    let prev = journal::chain_tip(root).map_err(|_| Reason::Schema)?;
    let parsed = journal::parse_migrate("migrate.json", &record).map_err(|_| Reason::Schema)?;
    let line = journal::seal(&journal::JournalEntry::Migrate(parsed), &prev);
    append_bundle(root, &line)?;
    append_studio(&state, &line)?;
    Ok(())
}

fn load_file_now(root: &Path) -> Result<Now, Reason> {
    if !root.join("state/save.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    let now_bytes = fs::read(root.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
    load_save(now_text).map_err(crate::diagnostic::reason_of)
}

fn journal_ahead(root: &Path, state_rev: u64) -> bool {
    let Ok(entries) = journal::read_entries(root) else {
        return false;
    };
    entries.iter().any(|entry| {
        !matches!(entry, journal::JournalEntry::Checkpoint(_))
            && entry.state_rev_after() > state_rev
    })
}

fn seal_tick(round_jsonl: &str, now_bytes: &[u8], root: &Path) -> Result<String, Reason> {
    let mut record: journal::TickRecord =
        serde_json::from_str(round_jsonl.trim_end()).map_err(|_| Reason::Schema)?;
    record.now_hash = hash_now(now_bytes);
    record.prev.clear();
    record.chain.clear();
    let prev = journal::chain_tip(root).map_err(|_| Reason::Schema)?;
    Ok(journal::seal(&journal::JournalEntry::Tick(record), &prev))
}

fn checkpoint_due(every: u64, tick_index: u64) -> bool {
    every > 0 && tick_index > 0 && tick_index.is_multiple_of(every)
}

fn append_bundle(root: &Path, payload: &str) -> Result<String, Reason> {
    let path = journal::append_path(root);
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

fn store_checkpoint(root: &Path, now_bytes: &[u8], state_rev: u64) -> Result<(), Reason> {
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
    atomic_publish(&state.join("save.yaml"), now_bytes)?;
    atomic_publish(&snap_dir.join(format!("{state_rev}.yaml")), now_bytes)?;
    close_live(root)
}

fn close_live(root: &Path) -> Result<(), Reason> {
    let Some(plain) = journal::live_plain(root) else {
        return Ok(());
    };
    let bytes = fs::read(&plain).map_err(|_| Reason::CommitConflict)?;
    let gz = gzip_path(&plain);
    let tmp = tmp_path(&gz);
    fs::write(&tmp, journal::gzip_bytes(&bytes)?).map_err(|_| Reason::CommitConflict)?;
    fsync_file(&tmp)?;
    rename(&tmp, &gz)?;
    fs::remove_file(&plain).map_err(|_| Reason::CommitConflict)?;
    if let Some(parent) = plain.parent() {
        fsync_dir(parent)?;
    }
    Ok(())
}

fn gzip_path(plain: &Path) -> PathBuf {
    let mut name = plain.file_name().unwrap_or_default().to_os_string();
    name.push(".gz");
    plain.with_file_name(name)
}

fn atomic_publish(target: &Path, bytes: &[u8]) -> Result<(), Reason> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|_| Reason::CommitConflict)?;
    }
    let tmp = tmp_path(target);
    {
        let mut file = File::create(&tmp).map_err(|_| Reason::CommitConflict)?;
        file.write_all(bytes).map_err(|_| Reason::CommitConflict)?;
    }
    fsync_file(&tmp)?;
    rename(&tmp, target)?;
    if let Some(parent) = target.parent() {
        fsync_dir(parent)?;
    }
    Ok(())
}

fn write_memories(state: &Path, memories: &[MemoryLine]) -> Result<(), Reason> {
    for (actor, body) in memory_bodies(state, memories)? {
        let path = state.join("memories").join(format!("{actor}.jsonl"));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| Reason::CommitConflict)?;
        }
        fs::write(&path, body).map_err(|_| Reason::CommitConflict)?;
    }
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

    use crate::draft::Draft;
    use crate::model::MemoryLine;
    use crate::store::durable::{self, Mark};
    use crate::world::World;

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
        let line = journal_line(0, 0, 1);
        durable::start_trace();
        commit_round(&mut world, draft, &line).unwrap();
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
        let memory_bytes = fs::read(&memory).unwrap();
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
        assert!(String::from_utf8(memory_bytes.clone())
            .unwrap()
            .contains("kept"));
        drop(world);
        let opened = World::open(&root).unwrap();
        assert_eq!(opened.now().state_rev, 0);
        assert_eq!(fs::read(&bundle).unwrap(), bundle_bytes);
        assert_eq!(fs::read(&memory).unwrap(), memory_bytes);
        assert!(!root.join("state/.txn").exists());

        let mut again = World::open(&root).unwrap();
        let next = Draft::begin(&again);
        durable::start_trace();
        commit_round(&mut again, next, &journal_line(1, 0, 1)).unwrap();
        let marks = durable::take_trace();
        assert_eq!(labels(&marks), ["file bundle"], "{}", show(&marks));
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

    fn journal_line(tick_index: u64, before: u64, after: u64) -> String {
        use crate::journal::{emit_entry, JournalEntry, TickRecord};
        emit_entry(&JournalEntry::Tick(TickRecord {
            tick_index,
            rules_rev: 7,
            state_rev_before: before,
            state_rev_after: after,
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
