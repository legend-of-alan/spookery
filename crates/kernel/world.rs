use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::canon::world_settings::Budget;
use crate::canon::{bind_inventory, bind_meters, load_canon, pack_rev, Canon};
use crate::diagnostic::{coded, locate_nth, reason_of, schema, Diagnostic};
use crate::error::Reason;
use crate::intent::Intent;
use crate::journal::{self, JournalEntry};
use crate::model::{sort_unique, MemoryLine, Now, Presence};
use crate::rules::RollReport;
use crate::store::codec::{emit_save, load_save};
use crate::store::recover::recover;
use crate::visibility::{Seen, Told};

use crate::draft::Draft;

pub struct World {
    pub(crate) root: PathBuf,
    canon: Arc<Canon>,
    pub(crate) now: Now,
    pub(crate) queue: Vec<Intent>,
    pub(crate) pending: Option<UncommittedRound>,
    rolls: Vec<RollReport>,
    summary: Vec<String>,
    lines: Vec<Told>,
    seen: Vec<Seen>,
    next_session: u64,
}

/// What one round showed its viewers.
pub struct Delivered {
    pub summary: Vec<String>,
    pub lines: Vec<Told>,
    pub seen: Vec<Seen>,
}

/// One round played and not yet written. A tick is only the time unit.
pub struct UncommittedRound {
    pub now: Now,
    pub round_jsonl: String,
    pub tick_index: u64,
    pub memories: Vec<MemoryLine>,
    pub paths: Vec<String>,
}

pub struct SessionStatus {
    pub humans: u32,
    pub actors_present: Vec<String>,
}

impl World {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn canon(&self) -> &Canon {
        &self.canon
    }

    pub fn now(&self) -> &Now {
        &self.now
    }

    /// Replace the in-memory state after a replay. This does not write `save.yaml`.
    pub fn install_replayed(&mut self, now: Now) {
        self.now = now;
    }

    pub fn presence(&self) -> &Presence {
        &self.now.presence
    }

    /// Remember who is seated. This does not write the save.
    pub fn set_presence(&mut self, mut presence: Presence) -> bool {
        sort_unique(&mut presence.actors_present);
        if presence == self.now.presence {
            return false;
        }
        self.now.presence = presence;
        true
    }

    /// The next seat id. Ids start at 1 and are not reused in this process.
    pub fn mint_session(&mut self) -> Result<String, Reason> {
        let id = format!("s{}", self.next_session);
        self.next_session = self.next_session.checked_add(1).ok_or(Reason::Schema)?;
        Ok(id)
    }

    pub fn budget(&self) -> &Budget {
        &self.canon.budget
    }

    pub fn queued_intents(&self) -> &[Intent] {
        &self.queue
    }

    pub fn queue(&mut self, intent: Intent) {
        self.queue.push(intent);
    }

    pub fn take_rolls(&mut self) -> Vec<RollReport> {
        std::mem::take(&mut self.rolls)
    }

    /// Change keys from the latest round. The GM sees every one.
    pub fn change_summary(&self) -> &[String] {
        &self.summary
    }

    /// Change keys each actor can see, from seated rounds since the last take.
    pub fn seen(&self) -> &[Seen] {
        &self.seen
    }

    pub fn take_lines(&mut self) -> Vec<Told> {
        std::mem::take(&mut self.lines)
    }

    pub fn take_seen(&mut self) -> Vec<Seen> {
        std::mem::take(&mut self.seen)
    }

    pub fn open(root: &Path) -> Result<Self, Reason> {
        load_world(root, false)
    }

    /// Open a temp pack for replay. A base from before migrate may not match `pack_rev`.
    pub fn open_for_replay(root: &Path) -> Result<Self, Reason> {
        load_world(root, true)
    }

    pub(crate) fn store_delivery(&mut self, delivered: Delivered, humans: u32) -> Vec<String> {
        self.summary = delivered.summary.clone();
        if humans == 0 {
            self.lines.clear();
            self.seen.clear();
        } else {
            self.lines.extend(delivered.lines);
            self.seen.extend(delivered.seen);
        }
        delivered.summary
    }

    pub(crate) fn hold_uncommitted(
        &mut self,
        now: Now,
        round_jsonl: String,
        tick_index: u64,
        memories: Vec<MemoryLine>,
        paths: Vec<String>,
        notices: Vec<RollReport>,
    ) {
        self.pending = Some(UncommittedRound {
            now,
            round_jsonl,
            tick_index,
            memories,
            paths,
        });
        self.rolls = notices;
    }

    pub(crate) fn note_committed(&mut self, notices: Vec<RollReport>, keep_queue: bool) {
        self.rolls = notices;
        if !keep_queue {
            self.queue.clear();
        }
    }
}

impl Draft {
    pub fn begin(world: &World) -> Self {
        Self::from_parts(
            Arc::clone(&world.canon),
            world.canon.budget,
            world.now.clone(),
        )
    }
}

pub fn session_status(now: &Now) -> SessionStatus {
    SessionStatus {
        humans: now.presence.humans,
        actors_present: now.presence.actors_present.clone(),
    }
}

fn load_world(root: &Path, replay: bool) -> Result<World, Reason> {
    if !root.join("pack.yaml").is_file() || !root.join("state/save.yaml").is_file() {
        return Err(Reason::NotInSpec);
    }
    recover(root)?;
    let canon = Arc::new(load_canon(root).map_err(reason_of)?);
    let now_bytes = fs::read(root.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
    let mut now = load_save(now_text).map_err(reason_of)?;
    let mut problems = save_problems(root, &now, &now_bytes);
    if replay || journal_continues(root, &now) {
        problems.retain(|diag| diag.code != Reason::PackRevMismatch.token());
    }
    if journal_continues(root, &now) {
        problems.retain(|diag| diag.code != Reason::HandEdit.token());
    }
    if !problems.is_empty() {
        return Err(reason_of(problems));
    }
    bind_meters(&canon, &mut now).map_err(reason_of)?;
    bind_inventory(&now).map_err(reason_of)?;
    let unknown = crate::canon::unknown_save_items(&canon, &now);
    if !unknown.is_empty() {
        return Err(reason_of(unknown));
    }
    Ok(World {
        root: root.to_path_buf(),
        canon,
        now,
        queue: Vec::new(),
        pending: None,
        rolls: Vec::new(),
        summary: Vec::new(),
        lines: Vec::new(),
        seen: Vec::new(),
        next_session: 1,
    })
}

/// The on-disk save is a checkpoint. Later journal records still have to be replayed.
fn journal_continues(root: &Path, now: &Now) -> bool {
    let Ok(entries) = journal::read_live(root) else {
        return false;
    };
    entries.iter().any(|entry| match entry {
        JournalEntry::Checkpoint(_) => false,
        _ => entry.state_rev_after() > now.state_rev,
    })
}

/// A migrate record past the checkpoint has already accepted the save's edit.
fn migrate_continues(root: &Path, now: &Now) -> bool {
    let Ok(entries) = journal::read_live(root) else {
        return false;
    };
    entries.iter().any(|entry| {
        matches!(entry, JournalEntry::Migrate(_)) && entry.state_rev_after() > now.state_rev
    })
}

/// Read the pack and report every loader and lint problem. Writes nothing.
pub fn check_pack(root: &Path) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let canon = match load_canon(root) {
        Ok(canon) => Some(canon),
        Err(more) => {
            diags.extend(more);
            None
        }
    };
    let now_path = root.join("state/save.yaml");
    let loaded = if now_path.is_file() {
        match fs::read(&now_path) {
            Ok(bytes) => match std::str::from_utf8(&bytes) {
                Ok(text) => match load_save(text) {
                    Ok(now) => Some((bytes, now)),
                    Err(more) => {
                        diags.extend(more);
                        None
                    }
                },
                Err(_) => {
                    diags.push(
                        schema(
                            "state/save.yaml",
                            "this file is not UTF-8 text",
                            "save it as UTF-8",
                        )
                        .with_path("schema"),
                    );
                    None
                }
            },
            Err(_) => {
                diags.push(
                    coded(
                        "state/save.yaml",
                        Reason::Schema,
                        "state/save.yaml could not be read",
                        "check that the file is readable",
                    )
                    .with_path("schema"),
                );
                None
            }
        }
    } else {
        None
    };
    if let Some((bytes, now)) = &loaded {
        let mut problems = save_problems(root, now, bytes);
        if migrate_continues(root, now) {
            problems.retain(|diag| {
                diag.code != Reason::PackRevMismatch.token()
                    && diag.code != Reason::HandEdit.token()
            });
        }
        diags.extend(problems);
        if let Some(canon) = &canon {
            let mut copy = now.clone();
            if let Err(more) = bind_meters(canon, &mut copy) {
                diags.extend(more);
            }
            if let Err(more) = bind_inventory(&copy) {
                diags.extend(more);
            }
            diags.extend(crate::canon::unknown_save_items(canon, &copy));
        }
    }
    if let Some(canon) = &canon {
        diags.extend(crate::lint::body_leaks(root, canon));
        diags.extend(crate::lint::message_text_leaks(root, canon));
        diags.extend(crate::map::unreachable(canon));
        diags.extend(crate::map::teleport_in_verbs(canon));
        diags.extend(crate::lint::trigger_cycles(canon));
        diags.extend(crate::lint::trigger_leaks(
            canon,
            loaded.as_ref().map(|(_, now)| now),
        ));
    }
    diags
}

fn save_problems(root: &Path, now: &Now, bytes: &[u8]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    match pack_rev(root) {
        Ok(rev) if now.pack_rev != rev => {
            let mut diag = coded(
                "state/save.yaml",
                Reason::PackRevMismatch,
                "pack_rev does not match the canon hash",
                "run spook migrate to store the current hash",
            )
            .with_path("pack_rev");
            if let Ok(text) = std::str::from_utf8(bytes) {
                if let Some((line, column)) = locate_nth(text, "pack_rev:", 0) {
                    diag = diag.at_line(line, Some(column));
                }
            }
            diags.push(diag);
        }
        Ok(_) => {}
        Err(_) => diags.push(
            coded(
                "state/save.yaml",
                Reason::Schema,
                "the canon files could not be hashed",
                "check that world/, cast/, and rules/ are readable",
            )
            .with_path("pack_rev"),
        ),
    }
    if now.actors.contains_key(crate::effect::WORLD) {
        diags.push(
            schema(
                "state/save.yaml",
                "actor id world is reserved",
                "rename that actor",
            )
            .with_path("actors.world"),
        );
    }
    match emit_save(now) {
        Ok(emitted) if emitted.as_bytes() != bytes => {
            let line = std::str::from_utf8(bytes)
                .ok()
                .and_then(|live| first_diff_line(live, &emitted));
            let mut diag = coded(
                "state/save.yaml",
                Reason::HandEdit,
                "this save does not match the canonical text",
                "run spook migrate to keep the edit, or restore the file",
            );
            diag = match line {
                Some(line) => diag.at_line(line, None),
                None => diag.with_path("now"),
            };
            diags.push(diag);
        }
        Ok(_) => {}
        Err(_) => diags.push(
            coded(
                "state/save.yaml",
                Reason::Schema,
                "the save could not be written back as YAML",
                "fix the values so they match the now format",
            )
            .with_path("now"),
        ),
    }
    diags
}

fn first_diff_line(live: &str, emitted: &str) -> Option<u32> {
    for (index, (left, right)) in live.lines().zip(emitted.lines()).enumerate() {
        if left != right {
            return Some(index as u32 + 1);
        }
    }
    if live.lines().count() != emitted.lines().count() {
        return Some(live.lines().count().min(emitted.lines().count()) as u32 + 1);
    }
    None
}
