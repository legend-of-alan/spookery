//! The host digest, and the player digest built from the view.

use std::path::Path;

use crate::error::Reason;
use crate::journal::{self, JournalEntry};
use crate::perceive::View;
use crate::store::codec::emit_effect_payload;
use crate::world::World;

pub fn host_summary(world: &World) -> Result<String, Reason> {
    let mut rows = effect_rows(world.root())?;
    for (id, clock) in &world.now().clocks {
        rows.push(clock_line(id, clock.value, clock.max, clock.paused));
    }
    Ok(rows_text(rows))
}

/// Notes addressed to this viewer, then their clocks. Which notes are on the
/// view is decided in `perceive`.
pub fn player_digest(view: &View) -> String {
    let id = view
        .viewer
        .split_once(':')
        .map(|(_, actor_id)| actor_id)
        .unwrap_or("");
    let mut rows = Vec::new();
    for message in &view.messages {
        if message.to.iter().any(|to| to == id) {
            rows.push(format!("message {} {}", message.id, message.text));
        }
    }
    for clock in &view.clocks {
        rows.push(clock_line(&clock.id, clock.value, clock.max, clock.paused));
    }
    rows_text(rows)
}

fn clock_line(id: &str, value: i64, max: i64, paused: bool) -> String {
    let state = if paused { "paused" } else { "running" };
    format!("clock {id} {value} {max} {state}")
}

fn rows_text(rows: Vec<String>) -> String {
    if rows.is_empty() {
        String::new()
    } else {
        rows.join("\n") + "\n"
    }
}

fn effect_rows(root: &Path) -> Result<Vec<String>, Reason> {
    let entries = journal::load_journal(root).map_err(|fault| {
        if fault.detail.contains("ILLEGAL_CLASS") {
            Reason::IllegalClass
        } else {
            Reason::Schema
        }
    })?;
    let mut rows = Vec::new();
    for entry in entries {
        let JournalEntry::Tick(record) = entry else {
            continue;
        };
        let tick_index = record.tick_index;
        for effect in record.effects {
            rows.push((
                tick_index,
                format!(
                    "effect {} {tick_index} {}",
                    effect.class_name(),
                    emit_effect_payload(&effect)
                ),
            ));
        }
    }
    rows.sort_by_key(|(tick_index, _)| *tick_index);
    Ok(rows.into_iter().map(|(_, row)| row).collect())
}
