use std::collections::BTreeSet;

use serde_json::{Map, Value};
use spookery_kernel::model::ActorLive;
use spookery_kernel::shard::Shard;
use spookery_kernel::tape::{append_session, read_session};
use spookery_kernel::Reason;

use super::protocol::{self, TapeLine};

#[derive(Debug)]
pub struct Spoken {
    pub tape: String,
    pub player_event: String,
    pub gm_event: Vec<u8>,
    pub tool: Value,
    pub recipients: Vec<String>,
}

enum Speech {
    Say { text: String },
    TalkActor { actor: String, text: String },
    TalkPlace { text: String },
}

pub fn speak(shard: &Shard, actor_id: &str, line: &str) -> Result<Spoken, Reason> {
    let speech = parse_line(line, actor_id)?;
    if !shard
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
    {
        return Err(Reason::Schema);
    }
    match &speech {
        Speech::TalkActor { actor, .. } => {
            if !shard.now().actors.contains_key(actor) {
                return Err(Reason::Schema);
            }
        }
        Speech::Say { .. } | Speech::TalkPlace { .. } => {}
    }
    if matches!(speech, Speech::TalkActor { .. } | Speech::TalkPlace { .. })
        && shard.now().presence.humans == 0
    {
        return Err(Reason::TableEmpty);
    }
    let speaker_place = shard
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::Schema)?
        .place()
        .to_string();
    let (address, place, actor, text) = match speech {
        Speech::Say { text } => ("table", None, None, text),
        Speech::TalkActor { actor, text } => ("actor", None, Some(actor), text),
        Speech::TalkPlace { text } => ("place", Some(speaker_place), None, text),
    };
    let recipients = recipients(
        shard,
        Some(actor_id),
        address,
        place.as_deref(),
        actor.as_deref(),
    );
    append_speech(
        shard,
        &format!("player:{actor_id}"),
        address,
        place,
        actor,
        recipients,
        text,
    )
}

/// GM `say`. `from` is the seat. A supplied `from` appends nothing.
pub fn speak_tool(shard: &Shard, arguments: Option<Map<String, Value>>) -> Result<Spoken, Reason> {
    let arguments = arguments.ok_or(Reason::Schema)?;
    let mut address = None;
    let mut place = None;
    let mut actor = None;
    let mut text = None;
    for (key, value) in &arguments {
        match key.as_str() {
            "from" => return Err(Reason::Schema),
            "address" => address = Some(value.as_str().ok_or(Reason::Schema)?.to_string()),
            "place" => {
                place = match value {
                    Value::Null => None,
                    Value::String(text) => Some(text.clone()),
                    _ => return Err(Reason::Schema),
                };
            }
            "actor" => {
                actor = match value {
                    Value::Null => None,
                    Value::String(text) => Some(text.clone()),
                    _ => return Err(Reason::Schema),
                };
            }
            "text" => text = Some(value.as_str().ok_or(Reason::Schema)?.to_string()),
            _ => return Err(Reason::Schema),
        }
    }
    let address = address.ok_or(Reason::Schema)?;
    let text = one_line(&text.ok_or(Reason::Schema)?)?.to_string();
    let (place, actor) = match address.as_str() {
        "table" => {
            if place.is_some() || actor.is_some() {
                return Err(Reason::Schema);
            }
            (None, None)
        }
        "place" => {
            let place = place.ok_or(Reason::Schema)?;
            if place.is_empty() || actor.is_some() {
                return Err(Reason::Schema);
            }
            (Some(place), None)
        }
        "actor" => {
            let actor = actor.ok_or(Reason::Schema)?;
            if actor.is_empty() || place.is_some() || !shard.now().actors.contains_key(&actor) {
                return Err(Reason::Schema);
            }
            (None, Some(actor))
        }
        _ => return Err(Reason::Schema),
    };
    let recipients = recipients(shard, None, &address, place.as_deref(), actor.as_deref());
    append_speech(shard, "gm", &address, place, actor, recipients, text)
}

pub fn last_tape_seq(root: &std::path::Path) -> Result<u64, Reason> {
    let mut last = 0u64;
    for tape in read_session(root)? {
        if let Some(stored) = protocol::parse_tape(&tape) {
            last = stored.seq;
        }
    }
    Ok(last)
}

/// Stored lines with `seq` greater than `since`, in file order.
pub fn tape_after(root: &std::path::Path, since: u64) -> Result<Vec<TapeLine>, Reason> {
    let mut out = Vec::new();
    for tape in read_session(root)? {
        if let Some(stored) = protocol::parse_tape(&tape) {
            if stored.seq > since {
                out.push(stored);
            }
        }
    }
    Ok(out)
}

fn append_speech(
    shard: &Shard,
    from: &str,
    address: &str,
    place: Option<String>,
    actor: Option<String>,
    recipients: Vec<String>,
    text: String,
) -> Result<Spoken, Reason> {
    let tick_index = shard.now().tick_index;
    let world_time = shard.now().world_time;
    let from = from.to_string();
    let address = address.to_string();
    let seq = append_session(shard.root(), |seq| {
        protocol::tape_body(&TapeLine {
            seq,
            from: from.clone(),
            address: address.clone(),
            place: place.clone(),
            actor: actor.clone(),
            recipients: recipients.clone(),
            text: text.clone(),
            tick_index,
            world_time,
        })
    })?;
    Ok(spoken_from(&TapeLine {
        seq,
        from,
        address,
        place,
        actor,
        recipients,
        text,
        tick_index,
        world_time,
    }))
}

pub fn player_scrollback(root: &std::path::Path, actor_id: &str) -> Result<Vec<String>, Reason> {
    let mut out = Vec::new();
    for tape in read_session(root)? {
        let Some(stored) = protocol::parse_tape(&tape) else {
            continue;
        };
        if stored.recipients.iter().any(|id| id == actor_id) {
            out.push(protocol::player_say_line(&stored));
        }
    }
    Ok(out)
}

fn spoken_from(stored: &TapeLine) -> Spoken {
    Spoken {
        tape: protocol::tape_body(stored),
        player_event: protocol::player_say_line(stored),
        gm_event: protocol::say_event(stored),
        tool: protocol::say_value(stored),
        recipients: stored.recipients.clone(),
    }
}

fn parse_line(line: &str, seated: &str) -> Result<Speech, Reason> {
    if line.is_empty() {
        return Err(Reason::Schema);
    }
    let (first, rest) = split_token(line);
    let (verb, rest) = if let Some(id) = first.strip_prefix("player:") {
        if id.is_empty() || id != seated {
            return Err(Reason::Schema);
        }
        if rest.is_empty() {
            return Err(Reason::Schema);
        }
        split_token(rest)
    } else {
        (first, rest)
    };
    if verb.is_empty() {
        return Err(Reason::Schema);
    }
    match verb {
        "say" => Ok(Speech::Say {
            text: one_line(rest)?.to_string(),
        }),
        "talk" => parse_talk(rest),
        _ => Err(Reason::NotInSpec),
    }
}

fn parse_talk(rest: &str) -> Result<Speech, Reason> {
    if rest.is_empty() {
        return Err(Reason::Schema);
    }
    let (target, text) = split_token(rest);
    if target.is_empty() {
        return Err(Reason::Schema);
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

fn one_line(text: &str) -> Result<&str, Reason> {
    if text.is_empty() || text.contains('\n') || text.chars().count() > 2000 {
        return Err(Reason::Schema);
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
    speaker: Option<&str>,
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
    if let Some(speaker) = speaker {
        ids.insert(speaker.to_string());
    }
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
