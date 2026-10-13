use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::canon::pack_rev;
use crate::codec::{canonicalize, emit_memory_line, emit_migrate_line, emit_now, load_now};
use crate::durable::{fsync_dir, fsync_file, tmp_path};
use crate::error::Reason;
use crate::journal::{self, hash_now};
use crate::lock::{lock_pack, lock_world};
use crate::model::{MemoryLine, Now, Presence};
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
    let now_bytes = emit_now(&draft.state)?.into_bytes();
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

/// `include_tick: false` for a presence change. No tick file and no clock advance.
/// Identical presence writes nothing and does not bump `state_rev`.
pub fn commit_presence(world: &mut World, presence: Presence) -> Result<bool, Reason> {
    let mut next = world.now.clone();
    next.presence = presence;
    canonicalize(&mut next);
    if next.presence == world.now.presence {
        return Ok(false);
    }
    let _lock = lock_world(world)?;
    let state_rev_before = next.state_rev;
    next.state_rev = next
        .state_rev
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    let now_bytes = emit_now(&next)?.into_bytes();
    let state = world.root.join("state");
    let pre = read_pre_now(&state)?;
    let record = journal::emit_presence(
        state_rev_before,
        next.state_rev,
        next.presence.humans,
        &next.presence.actors_present,
        &hash_now(&now_bytes),
    );
    let mut planned = vec![Planned {
        target: state.join("now.yaml"),
        bytes: now_bytes.clone(),
        is_now: true,
    }];
    push_base(&mut planned, &state, &pre, &now_bytes, false);
    planned.push(Planned {
        target: state
            .join("log")
            .join(journal::presence_file(next.state_rev)),
        bytes: record.into_bytes(),
        is_now: false,
    });
    write_commit(&state, &planned)?;
    world.now = next;
    Ok(true)
}

pub fn commit_pending(world: &mut World) -> Result<CommitOutcome, Reason> {
    let pending = world.pending.take().ok_or(Reason::CommitConflict)?;
    let mut now = pending.now;
    now.state_rev = now.state_rev.checked_add(1).ok_or(Reason::CommitConflict)?;
    canonicalize(&mut now);
    let now_bytes = emit_now(&now)?.into_bytes();
    let stamped = journal::stamp_now_hash(&pending.round_jsonl, &now_bytes)?;
    let tick_index = pending.tick_index;
    let log_path = format!("state/log/tick-{tick_index:04}.jsonl");
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
    if !root.join("pack.yaml").is_file() || !root.join("state/now.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    let now_bytes = fs::read(root.join("state/now.yaml")).map_err(|_| Reason::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
    let mut now = load_now(now_text).map_err(crate::diagnostic::reason_of)?;
    let canon = crate::canon::load_canon(root).map_err(crate::diagnostic::reason_of)?;
    crate::canon::bind_meters(&canon, &mut now).map_err(crate::diagnostic::reason_of)?;
    crate::canon::bind_inventory(&now).map_err(crate::diagnostic::reason_of)?;
    let pack_rev_after = pack_rev(root)?;
    let pack_rev_before = now.pack_rev.clone();
    let state_rev_before = now.state_rev;
    let _lock = lock_pack(root)?;
    now.pack_rev = pack_rev_after;
    now.state_rev = now.state_rev.checked_add(1).ok_or(Reason::CommitConflict)?;
    canonicalize(&mut now);
    let emitted = emit_now(&now)?.into_bytes();
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
        target: state.join("log").join(journal::migrate_file(now.state_rev)),
        bytes: record.into_bytes(),
        is_now: false,
    });
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

/// Replay writes `now.yaml` on a temp pack and does not append a journal record.
pub fn replay_write_now(world: &mut World, now: Now) -> Result<(), Reason> {
    let now_bytes = emit_now(&now)?.into_bytes();
    let state = world.root.join("state");
    let planned = vec![Planned {
        target: state.join("now.yaml"),
        bytes: now_bytes,
        is_now: true,
    }];
    write_commit(&state, &planned)?;
    world.now = now;
    Ok(())
}

fn read_pre_now(state: &Path) -> Result<Vec<u8>, Reason> {
    fs::read(state.join("now.yaml")).map_err(|_| Reason::NotInSpec)
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
