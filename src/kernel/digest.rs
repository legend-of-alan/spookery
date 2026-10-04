use std::fs;
use std::path::Path;

use crate::codec::{emit_effect_payload, parse_tick_effects};
use crate::error::Reason;
use crate::firewall::allows;
use crate::model::{Clock, ViewerId};
use crate::shard::Shard;

pub enum DigestViewer {
    Host,
    Player(String),
}

pub fn idle_digest(shard: &Shard, viewer: DigestViewer) -> Result<String, Reason> {
    let mut rows = Vec::new();
    match viewer {
        DigestViewer::Host => {
            rows.extend(effect_rows(shard.root())?);
            for (id, clock) in &shard.now().clocks {
                rows.push(clock_line(id, clock));
            }
        }
        DigestViewer::Player(id) => {
            let mut messages: Vec<_> = shard
                .now()
                .messages
                .iter()
                .filter(|message| message.to.iter().any(|to| to == &id))
                .collect();
            messages.sort_by(|left, right| left.id.cmp(&right.id));
            for message in messages {
                rows.push(format!("message {} {}", message.id, message.text));
            }
            let viewer = ViewerId::Player(id.clone());
            let mut rumors: Vec<_> = shard
                .now()
                .rumors
                .iter()
                .filter(|rumor| rumor.known_by.iter().any(|known| known == &id))
                .collect();
            rumors.sort_by(|left, right| left.id.cmp(&right.id));
            for rumor in rumors {
                let mut facts: Vec<_> = rumor
                    .fact_ids
                    .iter()
                    .filter(|fact| allows(shard.canon(), shard.now(), &viewer, fact))
                    .cloned()
                    .collect();
                facts.sort();
                facts.dedup();
                for fact in facts {
                    rows.push(format!("rumor {} {fact}", rumor.id));
                }
            }
            for (clock_id, clock) in &shard.now().clocks {
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
    let log_dir = root.join("state/log");
    if !log_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&log_dir).map_err(|_| Reason::Schema)? {
        let entry = entry.map_err(|_| Reason::Schema)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("tick-") && name.ends_with(".jsonl") {
            paths.push(entry.path());
        }
    }
    paths.sort();
    let mut rows = Vec::new();
    for path in paths {
        let text = fs::read_to_string(&path).map_err(|_| Reason::Schema)?;
        let (tick_index, effects) = parse_tick_effects(&text)?;
        for effect in effects {
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
