use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use spookery_kernel::model::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_kernel::shard::open_shard;
use spookery_kernel::Reason;

use crate::loop_::{run_recorded_tick, RecordedTick};
use crate::seed::seed_preimage;

pub fn replay_recorded(source: &Path, snapshot: &Path) -> Result<PathBuf, Reason> {
    let temp = std::env::temp_dir().join(format!("spook-replay-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    copy_dir(snapshot, &temp)?;
    let mut lines = read_tick_lines(source)?;
    lines.sort_by_key(|line| line.tick_index);
    let mut shard = open_shard(&temp)?;
    for line in lines {
        if shard.now().tick_index + 1 != line.tick_index {
            return Err(Reason::Schema);
        }
        let seed = seed_preimage(&shard.now().shard_id, line.tick_index);
        if line.seed != seed {
            return Err(Reason::Schema);
        }
        let wall = line.last_tick_at.clone();
        let rev_after = line.state_rev_after;
        run_recorded_tick(
            &mut shard,
            &wall,
            RecordedTick {
                intents: line.intents,
                presence_humans: line.presence_humans,
            },
        )?;
        if shard.now().state_rev != rev_after {
            return Err(Reason::Schema);
        }
    }
    Ok(temp)
}

struct TickLine {
    tick_index: u64,
    state_rev_after: u64,
    presence_humans: u32,
    seed: String,
    last_tick_at: String,
    intents: Vec<Intent>,
}

fn read_tick_lines(source: &Path) -> Result<Vec<TickLine>, Reason> {
    let dir = source.join("state/log");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut lines = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|_| Reason::Schema)? {
        let entry = entry.map_err(|_| Reason::Schema)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("tick-") && name.ends_with(".jsonl")) {
            continue;
        }
        let text = fs::read_to_string(entry.path()).map_err(|_| Reason::Schema)?;
        lines.push(parse_tick_line(&text)?);
    }
    Ok(lines)
}

fn parse_tick_line(text: &str) -> Result<TickLine, Reason> {
    let value: Value = serde_json::from_str(text.trim_end()).map_err(|_| Reason::Schema)?;
    let object = value.as_object().ok_or(Reason::Schema)?;
    let intents = object
        .get("intents")
        .and_then(Value::as_array)
        .ok_or(Reason::Schema)?;
    let mut parsed = Vec::new();
    for intent in intents {
        parsed.push(parse_intent(intent)?);
    }
    Ok(TickLine {
        tick_index: u64_field(object, "tick_index")?,
        state_rev_after: u64_field(object, "state_rev_after")?,
        presence_humans: u64_field(object, "presence_humans")?
            .try_into()
            .map_err(|_| Reason::Schema)?,
        seed: string_field(object, "seed")?,
        last_tick_at: string_field(object, "last_tick_at")?,
        intents: parsed,
    })
}

fn parse_intent(value: &Value) -> Result<Intent, Reason> {
    let object = value.as_object().ok_or(Reason::Schema)?;
    Ok(Intent {
        id: string_field(object, "id")?,
        tick_index: u64_field(object, "tick_index")?,
        actor_id: string_field(object, "actor_id")?,
        actor_kind: actor_kind(&string_field(object, "actor_kind")?)?,
        source: source(&string_field(object, "source")?)?,
        move_name: string_field(object, "move")?,
        target: parse_target(object.get("target").ok_or(Reason::Schema)?)?,
        args: parse_args(object.get("args").ok_or(Reason::Schema)?)?,
        rationale_code: string_field(object, "rationale_code")?,
    })
}

fn parse_target(value: &Value) -> Result<Target, Reason> {
    let object = value.as_object().ok_or(Reason::Schema)?;
    if object.is_empty() {
        return Ok(Target::Empty);
    }
    if object.len() != 1 {
        return Err(Reason::Schema);
    }
    if let Some(place) = object.get("place").and_then(Value::as_str) {
        return Ok(Target::Place(place.to_string()));
    }
    if let Some(actor) = object.get("actor").and_then(Value::as_str) {
        return Ok(Target::Actor(actor.to_string()));
    }
    if let Some(clock) = object.get("clock").and_then(Value::as_str) {
        return Ok(Target::Clock(clock.to_string()));
    }
    if let Some(rumor) = object.get("rumor").and_then(Value::as_str) {
        return Ok(Target::Rumor(rumor.to_string()));
    }
    Err(Reason::Schema)
}

fn parse_args(value: &Value) -> Result<IntentArgs, Reason> {
    let object = value.as_object().ok_or(Reason::Schema)?;
    let mut args = IntentArgs::default();
    for (key, item) in object {
        match key.as_str() {
            "to" => {
                let values = item.as_array().ok_or(Reason::Schema)?;
                let mut ids = Vec::new();
                for value in values {
                    ids.push(value.as_str().ok_or(Reason::Schema)?.to_string());
                }
                args.message_to = Some(ids);
            }
            _ => return Err(Reason::Schema),
        }
    }
    Ok(args)
}

fn actor_kind(text: &str) -> Result<ActorKind, Reason> {
    match text {
        "npc" => Ok(ActorKind::Npc),
        "player" => Ok(ActorKind::Player),
        "system" => Ok(ActorKind::System),
        _ => Err(Reason::Schema),
    }
}

fn source(text: &str) -> Result<Source, Reason> {
    match text {
        "kernel" => Ok(Source::Kernel),
        "player" => Ok(Source::Player),
        "system" => Ok(Source::System),
        _ => Err(Reason::Schema),
    }
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<String, Reason> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(Reason::Schema)
}

fn u64_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<u64, Reason> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(Reason::Schema)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), Reason> {
    fs::create_dir_all(to).map_err(|_| Reason::Schema)?;
    for entry in fs::read_dir(from).map_err(|_| Reason::Schema)? {
        let entry = entry.map_err(|_| Reason::Schema)?;
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|_| Reason::Schema)?;
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest).map_err(|_| Reason::Schema)?;
        }
    }
    Ok(())
}
