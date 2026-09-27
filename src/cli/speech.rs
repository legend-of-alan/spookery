use std::collections::BTreeSet;

use serde_json::Value;
use spookery_kernel::model::ActorLive;
use spookery_kernel::store::{append_session, read_session, Shard};
use spookery_kernel::Code;

#[derive(Debug)]
pub struct Spoken {
    pub tape: String,
    pub player_event: String,
    pub gm_event: Vec<u8>,
}

struct Stored {
    seq: u64,
    from: String,
    address: String,
    place: Option<String>,
    actor: Option<String>,
    recipients: Vec<String>,
    text: String,
    tick_index: u64,
    world_time: u64,
}

enum Speech {
    Say { text: String },
    TalkActor { actor: String, text: String },
    TalkPlace { text: String },
}

pub fn speak(shard: &Shard, actor_id: &str, line: &str) -> Result<Spoken, Code> {
    let speech = parse_line(line, actor_id)?;
    if !shard
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
    {
        return Err(Code::Schema);
    }
    match &speech {
        Speech::TalkActor { actor, .. } => {
            if !shard.now().actors.contains_key(actor) {
                return Err(Code::Schema);
            }
        }
        Speech::Say { .. } | Speech::TalkPlace { .. } => {}
    }
    if matches!(speech, Speech::TalkActor { .. } | Speech::TalkPlace { .. })
        && shard.now().presence.humans == 0
    {
        return Err(Code::LiveOnly);
    }
    let speaker_place = shard
        .now()
        .actors
        .get(actor_id)
        .ok_or(Code::Schema)?
        .place()
        .to_string();
    let (address, place, actor, text) = match speech {
        Speech::Say { text } => ("table", None, None, text),
        Speech::TalkActor { actor, text } => ("actor", None, Some(actor), text),
        Speech::TalkPlace { text } => ("place", Some(speaker_place), None, text),
    };
    let recipients = recipients(shard, actor_id, address, place.as_deref(), actor.as_deref());
    let tick_index = shard.now().tick_index;
    let world_time = shard.now().world_time;
    let from = format!("player:{actor_id}");
    let seq = append_session(shard.root(), |seq| {
        tape_json(
            seq,
            &from,
            address,
            place.as_deref(),
            actor.as_deref(),
            &recipients,
            &text,
            tick_index,
            world_time,
        )
    })?;
    let stored = Stored {
        seq,
        from,
        address: address.to_string(),
        place,
        actor,
        recipients,
        text,
        tick_index,
        world_time,
    };
    Ok(spoken_from(&stored))
}

pub fn player_scrollback(root: &std::path::Path, actor_id: &str) -> Result<Vec<String>, Code> {
    let mut out = Vec::new();
    for tape in read_session(root)? {
        let Some(stored) = parse_tape(&tape) else {
            continue;
        };
        if stored.recipients.iter().any(|id| id == actor_id) {
            out.push(player_event(&stored));
        }
    }
    Ok(out)
}

pub fn gm_events(root: &std::path::Path) -> Result<Vec<Vec<u8>>, Code> {
    let mut out = Vec::new();
    for tape in read_session(root)? {
        if let Some(stored) = parse_tape(&tape) {
            out.push(gm_event(&stored));
        }
    }
    Ok(out)
}

fn spoken_from(stored: &Stored) -> Spoken {
    Spoken {
        tape: tape_json(
            stored.seq,
            &stored.from,
            &stored.address,
            stored.place.as_deref(),
            stored.actor.as_deref(),
            &stored.recipients,
            &stored.text,
            stored.tick_index,
            stored.world_time,
        ),
        player_event: player_event(stored),
        gm_event: gm_event(stored),
    }
}

fn parse_line(line: &str, seated: &str) -> Result<Speech, Code> {
    if line.is_empty() {
        return Err(Code::Schema);
    }
    let (first, rest) = split_token(line);
    let (verb, rest) = if let Some(id) = first.strip_prefix("player:") {
        if id.is_empty() || id != seated {
            return Err(Code::Schema);
        }
        if rest.is_empty() {
            return Err(Code::Schema);
        }
        split_token(rest)
    } else {
        (first, rest)
    };
    if verb.is_empty() {
        return Err(Code::Schema);
    }
    match verb {
        "say" => Ok(Speech::Say {
            text: one_line(rest)?.to_string(),
        }),
        "talk" => parse_talk(rest),
        _ => Err(Code::NotInSpec),
    }
}

fn parse_talk(rest: &str) -> Result<Speech, Code> {
    if rest.is_empty() {
        return Err(Code::Schema);
    }
    let (target, text) = split_token(rest);
    if target.is_empty() {
        return Err(Code::Schema);
    }
    let text = one_line(text)?.to_string();
    if target == "place" {
        Ok(Speech::TalkPlace { text })
    } else {
        Ok(Speech::TalkActor {
            actor: target.to_string(),
            text,
        })
    }
}

fn one_line(text: &str) -> Result<&str, Code> {
    if text.is_empty() || text.contains('\n') || text.chars().count() > 2000 {
        return Err(Code::Schema);
    }
    Ok(text)
}

fn split_token(line: &str) -> (&str, &str) {
    match line.split_once(' ') {
        Some((token, rest)) => (token, rest),
        None => (line, ""),
    }
}

fn recipients(
    shard: &Shard,
    speaker: &str,
    address: &str,
    place: Option<&str>,
    actor: Option<&str>,
) -> Vec<String> {
    let mut ids = BTreeSet::new();
    match address {
        "table" => {
            for id in seated_players(shard) {
                ids.insert(id);
            }
        }
        "place" => {
            if let Some(place) = place {
                for id in seated_players(shard) {
                    if shard
                        .now()
                        .actors
                        .get(&id)
                        .is_some_and(|actor| actor.place() == place)
                    {
                        ids.insert(id);
                    }
                }
            }
        }
        "actor" => {
            if let Some(actor) = actor {
                if seated_players(shard).iter().any(|id| id == actor) {
                    ids.insert(actor.to_string());
                }
            }
        }
        _ => {}
    }
    ids.insert(speaker.to_string());
    ids.into_iter().collect()
}

fn seated_players(shard: &Shard) -> Vec<String> {
    shard
        .now()
        .presence
        .actors_present
        .iter()
        .filter(|id| shard.now().actors.get(*id).is_some_and(ActorLive::is_pc))
        .cloned()
        .collect()
}

fn tape_json(
    seq: u64,
    from: &str,
    address: &str,
    place: Option<&str>,
    actor: Option<&str>,
    recipients: &[String],
    text: &str,
    tick_index: u64,
    world_time: u64,
) -> String {
    let mut out = String::from("{\"seq\":");
    out.push_str(&seq.to_string());
    out.push_str(",\"from\":");
    push_string(&mut out, from);
    out.push_str(",\"address\":");
    push_string(&mut out, address);
    out.push_str(",\"place\":");
    push_optional(&mut out, place);
    out.push_str(",\"actor\":");
    push_optional(&mut out, actor);
    out.push_str(",\"recipients\":");
    push_strings(&mut out, recipients);
    out.push_str(",\"text\":");
    push_string(&mut out, text);
    out.push_str(",\"tick_index\":");
    out.push_str(&tick_index.to_string());
    out.push_str(",\"world_time\":");
    out.push_str(&world_time.to_string());
    out.push('}');
    out
}

fn player_event(stored: &Stored) -> String {
    let mut out = String::from("{\"kind\":\"say\",\"from\":");
    push_string(&mut out, &stored.from);
    out.push_str(",\"address\":");
    push_string(&mut out, &stored.address);
    out.push_str(",\"text\":");
    push_string(&mut out, &stored.text);
    out.push_str("}\n");
    out
}

fn gm_event(stored: &Stored) -> Vec<u8> {
    let mut params = String::from("{\"kind\":\"say\",\"seq\":");
    params.push_str(&stored.seq.to_string());
    params.push_str(",\"from\":");
    push_string(&mut params, &stored.from);
    params.push_str(",\"address\":");
    push_string(&mut params, &stored.address);
    params.push_str(",\"place\":");
    push_optional(&mut params, stored.place.as_deref());
    params.push_str(",\"actor\":");
    push_optional(&mut params, stored.actor.as_deref());
    params.push_str(",\"recipients\":");
    push_strings(&mut params, &stored.recipients);
    params.push_str(",\"text\":");
    push_string(&mut params, &stored.text);
    params.push('}');
    let mut out = String::from("{\"jsonrpc\":\"2.0\",\"method\":\"spook/event\",\"params\":");
    out.push_str(&params);
    out.push_str("}\n");
    out.into_bytes()
}

fn parse_tape(tape: &str) -> Option<Stored> {
    let value: Value = serde_json::from_str(tape).ok()?;
    let recipients = value.get("recipients")?.as_array()?;
    let mut ids = Vec::new();
    for id in recipients {
        ids.push(id.as_str()?.to_string());
    }
    Some(Stored {
        seq: value.get("seq")?.as_u64()?,
        from: value.get("from")?.as_str()?.to_string(),
        address: value.get("address")?.as_str()?.to_string(),
        place: optional_string(value.get("place")?)?,
        actor: optional_string(value.get("actor")?)?,
        recipients: ids,
        text: value.get("text")?.as_str()?.to_string(),
        tick_index: value.get("tick_index")?.as_u64()?,
        world_time: value.get("world_time")?.as_u64()?,
    })
}

fn optional_string(value: &Value) -> Option<Option<String>> {
    if value.is_null() {
        Some(None)
    } else {
        Some(Some(value.as_str()?.to_string()))
    }
}

fn push_optional(out: &mut String, value: Option<&str>) {
    match value {
        Some(value) => push_string(out, value),
        None => out.push_str("null"),
    }
}

fn push_strings(out: &mut String, values: &[String]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_string(out, value);
    }
    out.push(']');
}

fn push_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(other))),
            other => out.push(other),
        }
    }
    out.push('"');
}
