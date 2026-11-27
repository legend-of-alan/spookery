//! The save file's load and check, and the read-only check of a whole pack that `spook check` runs.

use std::path::Path;
use std::sync::Arc;

use crate::canon::{self, Canon};
use crate::diagnostic::{self, error, schema_error, Code, Diagnostic};
use crate::ecs::{self, Save};
use crate::store::journal;
use crate::store::records::Record;
use crate::store::save;
use crate::Reason;

/// Hash of the newest migrate in `live` past the checkpoint, or the save's own hash.
fn read_tip<'rec>(live: &'rec [Record], save: &'rec Save) -> &'rec str {
    live.iter()
        .rev()
        .filter(|entry| entry.state_rev_after() > save.state_rev)
        .find_map(|entry| match entry {
            Record::Migrate(record) => Some(record.pack_rev_after.as_str()),
            _ => None,
        })
        .unwrap_or(&save.pack_rev)
}

/// `Some` when the canon files are not the journal tip's hash.
fn find_rev_mismatch(root: &Path, tip: &str, bytes: &[u8]) -> Option<Diagnostic> {
    match canon::hash(root) {
        Ok(rev) if tip != rev => {
            let mut diagnostic = error(
                "state/save.yaml",
                Reason::PackRevMismatch,
                "pack_rev does not match the canon hash",
                "run spook migrate to store the current hash",
            )
            .at_key_path("pack_rev");
            if let Ok(text) = std::str::from_utf8(bytes) {
                diagnostic =
                    diagnostic.at_file_location(diagnostic::find_nth(text, "pack_rev:", 0));
            }
            Some(diagnostic)
        }
        Ok(_) => None,
        Err(_) => Some(
            error(
                "state/save.yaml",
                Reason::Schema,
                "the canon files could not be hashed",
                "check that world/, cast/, and rules/ are readable",
            )
            .at_key_path("pack_rev"),
        ),
    }
}

/// The on-disk save is a checkpoint. Later live records still have to be replayed.
fn journal_continues(live: &[Record], save: &Save) -> bool {
    live.iter().any(|entry| match entry {
        Record::Checkpoint(_) => false,
        _ => entry.state_rev_after() > save.state_rev,
    })
}

/// The save in `bytes`, or every diagnostic for it. A hand edit is refused unless a later
/// migrate record accepted it. A checkpoint save is not the tip, so only a save that no
/// record continues is checked against `canon`.
pub(crate) fn load(
    root: &Path,
    canon: Option<&Arc<Canon>>,
    bytes: &[u8],
) -> Result<Save, Vec<Diagnostic>> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        vec![schema_error(
            "state/save.yaml",
            "this file is not UTF-8 text",
            "save it as UTF-8",
        )
        .at_key_path("schema")]
    })?;
    let save = save::parse(text)?;
    let live = journal::read_live_records(root).unwrap_or_default();
    let mut diagnostics = diagnose_save_text(&save, bytes);
    if let Some(diagnostic) = find_rev_mismatch(root, read_tip(&live, &save), bytes) {
        diagnostics.insert(0, diagnostic);
    }
    let migrate_follows = live.iter().any(|entry| {
        matches!(entry, Record::Migrate(_)) && entry.state_rev_after() > save.state_rev
    });
    let continues = journal_continues(&live, &save);
    let is_hand_edit = |diagnostic: &Diagnostic| diagnostic.code == Code::Error(Reason::HandEdit);
    if migrate_follows {
        diagnostics.retain(|diagnostic| !is_hand_edit(diagnostic));
    } else if !diagnostics.iter().any(is_hand_edit) {
        match journal::edits_mark(root, save.state_rev, bytes) {
            Ok(false) => {}
            Ok(true) => diagnostics.push(
                error(
                    "state/save.yaml",
                    Reason::HandEdit,
                    "this save differs from the checkpoint mark it claims",
                    "restore the file, or run spook migrate to keep the edit if no round follows it",
                )
                .at_key_path("save"),
            ),
            Err(err) => diagnostics.push(
                error(
                    "state/journal/mark.json",
                    err.reason,
                    "the checkpoint mark could not be read",
                    "check that state/journal/mark.json is readable",
                )
                .at_key_path("schema"),
            ),
        }
    }
    if let Some(canon) = canon.filter(|_| !continues) {
        if let Err(more) = save::check(&ecs::World::new(save.clone(), Arc::clone(canon))) {
            diagnostics.extend(more);
        }
    }
    if diagnostics.is_empty() {
        Ok(save)
    } else {
        Err(diagnostics)
    }
}

/// Read the pack and report every loader and lint diagnostic. Writes nothing.
pub fn run(root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let canon = match canon::load(root) {
        Ok(canon) => Some(Arc::new(canon)),
        Err(more) => {
            diagnostics.extend(more);
            None
        }
    };
    let mut save = None;
    match journal::read_save(&journal::save_path(root)) {
        Ok(bytes) => match load(root, canon.as_ref(), &bytes) {
            Ok(loaded) => save = Some(loaded),
            Err(more) => diagnostics.extend(more),
        },
        Err(Reason::NotInSpec) => {}
        Err(reason) => diagnostics.push(
            error(
                "state/save.yaml",
                reason,
                "state/save.yaml could not be read",
                "check that the file is readable",
            )
            .at_key_path("schema"),
        ),
    }
    if let Some(canon) = &canon {
        let world = save.map(|save| ecs::World::new(save, Arc::clone(canon)));
        let saved_flags = world
            .as_ref()
            .map(save::find_saved_flags)
            .unwrap_or_default();
        diagnostics.extend(crate::canon::lint::run(canon, &saved_flags));
    }
    diagnostics
}

/// The reserved actor id `world` in the save, and a save that is not its own canonical text.
fn diagnose_save_text(save: &Save, bytes: &[u8]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let saved = serde_json::to_value(save).unwrap_or_default();
    if saved["actors"]["control_type"].get("world").is_some() {
        let diagnostic = schema_error(
            "state/save.yaml",
            "actor id world is reserved",
            "rename that actor",
        );
        diagnostics.push(diagnostic.at_key_path("actors.world"));
    }
    match save::emit(save) {
        Ok(emitted) if emitted.as_bytes() != bytes => {
            let line = std::str::from_utf8(bytes).ok().and_then(|live| {
                find_first_diverged_line(live, &emitted).map(|(line_no, _)| line_no)
            });
            let mut diagnostic = error(
                "state/save.yaml",
                Reason::HandEdit,
                "this save does not match the canonical text",
                "run spook migrate to keep the edit, or restore the file",
            );
            diagnostic = match line {
                Some(line) => diagnostic.at_line(line, None),
                None => diagnostic.at_key_path("save"),
            };
            diagnostics.push(diagnostic);
        }
        Ok(_) => {}
        Err(_) => diagnostics.push(
            error(
                "state/save.yaml",
                Reason::Schema,
                "the save could not be written back as YAML",
                "fix the values so they match the save format",
            )
            .at_key_path("save"),
        ),
    }
    diagnostics
}

/// First 1-based line number and line text from `live` that differs from `expected`.
pub(crate) fn find_first_diverged_line(live: &str, expected: &str) -> Option<(u32, String)> {
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
