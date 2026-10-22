//! An open pack, and the check that reads one without writing.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::canon::tick_settings::Budget;
use crate::canon::{check_save, load_canon, pack_rev, Canon};
use crate::diagnostic::{coded, locate_nth, reason_of, schema, Diagnostic};
use crate::error::Reason;
use crate::intent::Intent;
use crate::journal::{self, JournalEntry};
use crate::model::{sort_unique, Now, Presence};
use crate::store::codec::{emit_save, load_save};
use crate::store::recover::recover;

use crate::draft::Draft;

pub struct World {
    pub(crate) root: PathBuf,
    canon: Arc<Canon>,
    pub(crate) now: Now,
    pub(crate) queue: Vec<Intent>,
    next_session: u64,
    warning: Option<&'static str>,
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

    /// A committed round whose checkpoint file was not finished. The host prints it.
    pub fn take_warning(&mut self) -> Option<&'static str> {
        self.warning.take()
    }

    pub(crate) fn note_warning(&mut self, text: &'static str) {
        self.warning = Some(text);
    }

    /// Open the pack. A save that later journal records continue is a checkpoint,
    /// and resume checks the tip instead.
    pub fn open(root: &Path) -> Result<Self, Reason> {
        if !root.join("pack.yaml").is_file() || !root.join("state/save.yaml").is_file() {
            return Err(Reason::NotInSpec);
        }
        recover(root)?;
        let canon = Arc::new(load_canon(root).map_err(reason_of)?);
        let now_bytes = fs::read(root.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
        let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
        let now = load_save(now_text).map_err(reason_of)?;
        let mut problems = save_problems(&now, &now_bytes);
        if let Some(diag) = pack_rev_problem(root, &now, &now_bytes) {
            problems.insert(0, diag);
        }
        let continues = journal_continues(root, &now);
        if continues {
            problems.retain(|diag| diag.code != Reason::HandEdit.token());
        }
        if !problems.is_empty() {
            return Err(reason_of(problems));
        }
        if !continues {
            check_save(&canon, &now).map_err(reason_of)?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            canon,
            now,
            queue: Vec::new(),
            next_session: 1,
            warning: None,
        })
    }

    /// The pack's canon with a save from memory, for replay. Nothing is checked:
    /// each journal record's hash proves the state it reaches.
    pub fn open_for_replay(pack: &Path, now: Now) -> Result<Self, Reason> {
        Ok(Self {
            root: pack.to_path_buf(),
            canon: Arc::new(load_canon(pack).map_err(reason_of)?),
            now,
            queue: Vec::new(),
            next_session: 1,
            warning: None,
        })
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

/// Hash of the newest migrate past the checkpoint, or the save's own hash.
fn tip_pack_rev(root: &Path, now: &Now) -> String {
    let Ok(entries) = journal::read_live(root) else {
        return now.pack_rev.clone();
    };
    for entry in entries.iter().rev() {
        if entry.state_rev_after() <= now.state_rev {
            continue;
        }
        if let JournalEntry::Migrate(record) = entry {
            return record.pack_rev_after.clone();
        }
    }
    now.pack_rev.clone()
}

/// `Some` when the canon files are not the journal tip's hash.
fn pack_rev_problem(root: &Path, now: &Now, bytes: &[u8]) -> Option<Diagnostic> {
    let tip = tip_pack_rev(root, now);
    match pack_rev(root) {
        Ok(rev) if tip != rev => {
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
            Some(diag)
        }
        Ok(_) => None,
        Err(_) => Some(
            coded(
                "state/save.yaml",
                Reason::Schema,
                "the canon files could not be hashed",
                "check that world/, cast/, and rules/ are readable",
            )
            .with_path("pack_rev"),
        ),
    }
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
        let mut problems = save_problems(now, bytes);
        let migrating = migrate_continues(root, now);
        if migrating {
            problems.retain(|diag| diag.code != Reason::HandEdit.token());
        }
        if let Some(diag) = pack_rev_problem(root, now, bytes) {
            problems.insert(0, diag);
        }
        diags.extend(problems);
        // A checkpoint save is not the tip. Resume checks the tip when it opens.
        if let Some(canon) = canon.as_ref().filter(|_| !journal_continues(root, now)) {
            if let Err(more) = check_save(canon, now) {
                diags.extend(more);
            }
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

fn save_problems(now: &Now, bytes: &[u8]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
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
                .and_then(|live| first_diff_line(live, &emitted).map(|(line_no, _)| line_no));
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

/// First 1-based line number and line text from `live` that differs from `expected`.
pub fn first_diff_line(live: &str, expected: &str) -> Option<(u32, String)> {
    for (index, (left, right)) in live.lines().zip(expected.lines()).enumerate() {
        if left != right {
            return Some(((index + 1) as u32, left.to_string()));
        }
    }
    let live_count = live.lines().count();
    let expected_count = expected.lines().count();
    if live_count != expected_count {
        let number = live_count.min(expected_count) + 1;
        let line = live
            .lines()
            .nth(number - 1)
            .or_else(|| expected.lines().nth(number - 1))
            .unwrap_or("")
            .to_string();
        return Some((number as u32, line));
    }
    if live.as_bytes() != expected.as_bytes() {
        return Some((1, live.lines().next().unwrap_or("").to_string()));
    }
    None
}
