//! The host digest, and the player digest built from the view.

use crate::effect::Effect;
use crate::error::Reason;
use crate::intent::ViewerId;
use crate::journal::{self, JournalEntry};
use crate::perceive::View;
use crate::store::codec::emit_effect_payload;
use crate::world::World;

pub fn host_summary(world: &World) -> Result<String, Reason> {
    let entries = journal::read_verified(world.root()).map_err(|fault| {
        if fault.detail.contains("ILLEGAL_CLASS") {
            Reason::IllegalClass
        } else {
            Reason::Schema
        }
    })?;
    let mut effects = Vec::new();
    for entry in &entries {
        let JournalEntry::Tick(record) = entry else {
            continue;
        };
        for effect in &record.effects {
            effects.push((record.tick_index, effect));
        }
    }
    Ok(host_lines(world, &effects))
}

/// Effect rows, then the clocks. Does not read the journal.
pub fn host_lines(world: &World, effects: &[(u64, &Effect)]) -> String {
    let mut rows = Vec::new();
    for (tick_index, effect) in effects {
        rows.push(format!(
            "effect {} {tick_index} {}",
            effect.class_name(),
            emit_effect_payload(effect)
        ));
    }
    for (id, clock) in &world.now().clocks {
        rows.push(clock_line(id, clock.value, clock.max, clock.paused));
    }
    join_rows(rows)
}

/// Notes addressed to this viewer, then their clocks. Which notes are on the
/// view is decided in `perceive`.
pub fn player_digest(view: &View) -> Result<String, Reason> {
    let viewer = ViewerId::parse(&view.viewer)?;
    let id = viewer.actor_id();
    let mut rows = Vec::new();
    for message in &view.messages {
        if message.to.iter().any(|to| to == id) {
            rows.push(format!("message {} {}", message.id, message.text));
        }
    }
    for clock in &view.clocks {
        rows.push(clock_line(&clock.id, clock.value, clock.max, clock.paused));
    }
    Ok(join_rows(rows))
}

fn clock_line(id: &str, value: i64, max: i64, paused: bool) -> String {
    let state = if paused { "paused" } else { "running" };
    format!("clock {id} {value} {max} {state}")
}

fn join_rows(rows: Vec<String>) -> String {
    if rows.is_empty() {
        String::new()
    } else {
        rows.join("\n") + "\n"
    }
}
