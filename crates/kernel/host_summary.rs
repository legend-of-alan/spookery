//! The host digest, and the shorter digest a player sees.

use std::path::Path;

use crate::error::Reason;
use crate::journal::{self, JournalEntry};
use crate::model::Clock;
use crate::store::codec::emit_effect_payload;
use crate::world::World;

pub enum DigestViewer {
    Host,
    Player(String),
}

pub fn host_summary(world: &World, viewer: DigestViewer) -> Result<String, Reason> {
    let mut rows = Vec::new();
    match viewer {
        DigestViewer::Host => {
            rows.extend(effect_rows(world.root())?);
            for (id, clock) in &world.now().clocks {
                rows.push(clock_line(id, clock));
            }
        }
        DigestViewer::Player(id) => {
            let mut messages: Vec<_> = world
                .now()
                .messages
                .iter()
                .filter(|message| message.to.iter().any(|to| to == &id))
                .collect();
            messages.sort_by(|left, right| left.id.cmp(&right.id));
            for message in messages {
                rows.push(format!("message {} {}", message.id, message.text));
            }
            for (clock_id, clock) in &world.now().clocks {
                if clock.owners.iter().any(|owner| owner == &id) {
                    rows.push(clock_line(clock_id, clock));
                }
            }
        }
    }
    if rows.is_empty() {
        Ok(String::new())
    } else {
        Ok(rows.join("\n") + "\n")
    }
}

fn clock_line(id: &str, clock: &Clock) -> String {
    let state = if clock.paused { "paused" } else { "running" };
    format!("clock {id} {} {} {state}", clock.value, clock.max)
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
