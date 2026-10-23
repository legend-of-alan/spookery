//! The host digest.

use crate::effect::{payload_json, Effect};
use crate::error::Reason;
use crate::journal::{self, JournalEntry};
use crate::world::World;

pub fn host_summary(world: &World) -> Result<String, Reason> {
    let entries = journal::read_verified(world.root()).map_err(|fault| fault.reason)?;
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
            payload_json(effect)
        ));
    }
    for (id, clock) in &world.now().clocks {
        rows.push(clock_line(id, clock.value, clock.max, clock.paused));
    }
    if rows.is_empty() {
        String::new()
    } else {
        rows.join("\n") + "\n"
    }
}

fn clock_line(id: &str, value: i64, max: i64, paused: bool) -> String {
    let state = if paused { "paused" } else { "running" };
    format!("clock {id} {value} {max} {state}")
}
