use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::canon::pack_rev;
use crate::codec::{canonicalize, emit_memory_line, emit_migrate_line, emit_now, load_now};
use crate::durable::{fsync_dir, fsync_file, tmp_path};
use crate::error::Reason;
use crate::lock::{lock_pack, lock_shard};
use crate::model::{MemoryLine, Presence};
use crate::shard::{Shard, Txn};

pub struct CommitOutcome {
    pub state_rev: u64,
    pub diff_summary: Vec<String>,
    pub log_path: String,
}

pub fn tick_commit(shard: &mut Shard, mut txn: Txn, tick_jsonl: &str) -> Result<(), Reason> {
    txn.staging.state_rev = txn
        .staging
        .state_rev
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    canonicalize(&mut txn.staging);
    let now_bytes = emit_now(&txn.staging)?.into_bytes();
    let tick_index = txn.staging.tick_index;
    let state = shard.root.join("state");
    let planned = plan_commit(
        &state,
        now_bytes,
        Some((tick_index, tick_jsonl.as_bytes())),
        &txn.memories,
    )?;
    write_commit(&state, &planned)?;
    shard.now = txn.staging;
    Ok(())
}

/// `include_tick: false` for a presence change. No tick file and no clock advance.
/// Identical presence writes nothing and does not bump `state_rev`.
pub fn commit_presence(shard: &mut Shard, presence: Presence) -> Result<bool, Reason> {
    let mut next = shard.now.clone();
    next.presence = presence;
    canonicalize(&mut next);
    if next.presence == shard.now.presence {
        return Ok(false);
    }
    let _lock = lock_shard(shard)?;
    next.state_rev = next
        .state_rev
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    let now_bytes = emit_now(&next)?.into_bytes();
    let state = shard.root.join("state");
    let planned = vec![Planned {
        target: state.join("now.yaml"),
        bytes: now_bytes,
        is_now: true,
    }];
    write_commit(&state, &planned)?;
    shard.now = next;
    Ok(true)
}

pub fn commit_pending(shard: &mut Shard) -> Result<CommitOutcome, Reason> {
    let pending = shard.pending.take().ok_or(Reason::CommitConflict)?;
    let mut now = pending.now;
    now.state_rev = now.state_rev.checked_add(1).ok_or(Reason::CommitConflict)?;
    canonicalize(&mut now);
    let now_bytes = emit_now(&now)?.into_bytes();
    let tick_index = pending.tick_index;
    let log_path = format!("state/log/tick-{tick_index:04}.jsonl");
    let state = shard.root.join("state");
    let planned = plan_commit(
        &state,
        now_bytes,
        Some((tick_index, pending.tick_jsonl.as_bytes())),
        &pending.memories,
    )?;
    write_commit(&state, &planned)?;
    shard.now = now;
    shard.queue.clear();
    Ok(CommitOutcome {
        state_rev: shard.now.state_rev,
        diff_summary: pending.paths,
        log_path,
    })
}

pub fn migrate_pack(root: &Path) -> Result<(), Reason> {
    if !root.join("pack.yaml").is_file() || !root.join("state/now.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    let now_bytes = fs::read(root.join("state/now.yaml")).map_err(|_| Reason::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
    let mut now = load_now(now_text)?;
    let canon = crate::canon::load_canon(root)?;
    crate::canon::bind_meters(&canon, &mut now)?;
    crate::canon::bind_inventory(&now)?;
    let pack_rev_after = pack_rev(root)?;
    let pack_rev_before = now.pack_rev.clone();
    let state_rev_before = now.state_rev;
    let _lock = lock_pack(root)?;
    now.pack_rev = pack_rev_after;
    now.state_rev = now.state_rev.checked_add(1).ok_or(Reason::CommitConflict)?;
    canonicalize(&mut now);
    let emitted = emit_now(&now)?.into_bytes();
    let state = root.join("state");
    let planned = plan_commit(&state, emitted, None, &[])?;
    write_commit(&state, &planned)?;
    let line = format!(
        "{}\n",
        emit_migrate_line(
            &pack_rev_before,
            &now.pack_rev,
            state_rev_before,
            now.state_rev
        )
    );
    append_studio(&state, &line)?;
    Ok(())
}

fn append_studio(state: &Path, line: &str) -> Result<(), Reason> {
    let log_dir = state.join("log");
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

fn plan_commit(
    state: &Path,
    now_bytes: Vec<u8>,
    tick: Option<(u64, &[u8])>,
    memories: &[MemoryLine],
) -> Result<Vec<Planned>, Reason> {
    let mut planned = Vec::new();
    planned.push(Planned {
        target: state.join("now.yaml"),
        bytes: now_bytes,
        is_now: true,
    });
    if let Some((tick_index, tick_jsonl)) = tick {
        planned.push(Planned {
            target: state.join(format!("log/tick-{tick_index:04}.jsonl")),
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
    for item in planned {
        if item.target.is_file() {
            let rel = item
                .target
                .strip_prefix(state)
                .map_err(|_| Reason::CommitConflict)?;
            let dest = prev.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|_| Reason::CommitConflict)?;
            }
            fs::copy(&item.target, &dest).map_err(|_| Reason::CommitConflict)?;
            fsync_file(&dest)?;
        }
    }
    fsync_dir(&prev)?;
    fsync_dir(&txn_dir)?;
    let complete = txn_dir.join("COMPLETE");
    {
        let file = File::create(&complete).map_err(|_| Reason::CommitConflict)?;
        file.sync_all().map_err(|_| Reason::CommitConflict)?;
    }
    fsync_dir(&txn_dir)?;

    let mut temps = Vec::new();
    for item in planned {
        let tmp = tmp_path(&item.target);
        if let Some(parent) = tmp.parent() {
            fs::create_dir_all(parent).map_err(|_| Reason::CommitConflict)?;
        }
        {
            let mut file = File::create(&tmp).map_err(|_| Reason::CommitConflict)?;
            file.write_all(&item.bytes)
                .map_err(|_| Reason::CommitConflict)?;
            file.sync_all().map_err(|_| Reason::CommitConflict)?;
        }
        if let Some(parent) = item.target.parent() {
            fsync_dir(parent)?;
        }
        temps.push((tmp, item));
    }
    for (tmp, item) in &temps {
        if !item.is_now {
            fs::rename(tmp, &item.target).map_err(|_| Reason::CommitConflict)?;
        }
    }
    for (tmp, item) in &temps {
        if item.is_now {
            fs::rename(tmp, &item.target).map_err(|_| Reason::CommitConflict)?;
        }
    }
    fsync_dir(state)?;
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
