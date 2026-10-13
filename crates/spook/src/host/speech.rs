use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use spookery_kernel::model::ActorLive;
use spookery_kernel::store::talk_log::{append_session, read_session};
use spookery_kernel::world::World;
use spookery_kernel::Reason;

use super::frames::{self, TapeLine};
use super::input::split_token;

#[derive(Debug)]
pub struct Spoken {
    pub tape: String,
    pub player_event: String,
    pub gm_event: Vec<u8>,
    pub tool: Value,
    pub recipients: Vec<String>,
}

enum Speech {
    Table { text: String },
    Actor { actor: String, text: String },
    Place { text: String },
}

pub fn speak(world: &World, actor_id: &str, line: &str) -> Result<Spoken, Reason> {
    let speech = parse_line(line, actor_id, &world.now().actors)?;
    if !world
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
    {
        return Err(Reason::Schema);
    }
    let speaker_place = world
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::Schema)?
        .place()
        .to_string();
    let (address, place, actor, text) = match speech {
        Speech::Table { text } => ("table", None, None, text),
        Speech::Actor { actor, text } => ("actor", None, Some(actor), text),
        Speech::Place { text } => ("place", Some(speaker_place), None, text),
    };
    let recipients = recipients(
        world,
        Some(actor_id),
        address,
        place.as_deref(),
        actor.as_deref(),
    );
    append_speech(
        world,
        &format!("player:{actor_id}"),
        address,
        place,
        actor,
        recipients,
        text,
    )
}

/// GM `say`. `from` is the seat. A supplied `from` appends nothing.
pub fn speak_tool(world: &World, arguments: Option<Map<String, Value>>) -> Result<Spoken, Reason> {
    let call = crate::protocol::read_say_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let address = call.address;
    let place = call.place;
    let actor = call.actor;
    let text = one_line(&call.text)?.to_string();
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
            if actor.is_empty() || place.is_some() || !world.now().actors.contains_key(&actor) {
                return Err(Reason::Schema);
            }
            (None, Some(actor))
        }
        _ => return Err(Reason::Schema),
    };
    let recipients = recipients(world, None, &address, place.as_deref(), actor.as_deref());
    append_speech(world, "gm", &address, place, actor, recipients, text)
}

pub fn last_tape_seq(root: &std::path::Path) -> Result<u64, Reason> {
    spookery_kernel::store::talk_log::session_end_seq(root)
}

/// Stored lines with `seq` greater than `since`, in file order, inside the tape window.
pub fn tape_after(root: &std::path::Path, since: u64) -> Result<Vec<TapeLine>, Reason> {
    let mut out = Vec::new();
    for tape in read_session(root, since, None)? {
        out.push(frames::parse_tape(&tape)?);
    }
    Ok(out)
}

fn append_speech(
    world: &World,
    from: &str,
    address: &str,
    place: Option<String>,
    actor: Option<String>,
    recipients: Vec<String>,
    text: String,
) -> Result<Spoken, Reason> {
    let tick_index = world.now().tick_index;
    let world_time = world.now().world_time;
    let from = from.to_string();
    let address = address.to_string();
    let seq = append_session(world.root(), |seq| {
        frames::tape_body(&TapeLine {
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
    for tape in read_session(root, 0, Some(256))? {
        let stored = frames::parse_tape(&tape)?;
        if stored.recipients.iter().any(|id| id == actor_id) {
            out.push(frames::player_say_line(&stored));
        }
    }
    Ok(out)
}

fn spoken_from(stored: &TapeLine) -> Spoken {
    Spoken {
        tape: frames::tape_body(stored),
        player_event: frames::player_say_line(stored),
        gm_event: frames::say_event(stored),
        tool: frames::say_value(stored),
        recipients: stored.recipients.clone(),
    }
}

fn parse_line(
    line: &str,
    seated: &str,
    actors: &BTreeMap<String, ActorLive>,
) -> Result<Speech, Reason> {
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
    if verb != "say" {
        return Err(Reason::NotInSpec);
    }
    // `place`, or an actor id, is the address when more words follow.
    // One word is table text, so a name by itself still reaches the table.
    if rest.is_empty() {
        return Err(Reason::Schema);
    }
    let (target, text) = split_token(rest);
    if !text.is_empty() && target == "place" {
        return Ok(Speech::Place {
            text: one_line(text)?.to_string(),
        });
    }
    if !text.is_empty() && actors.contains_key(target) {
        return Ok(Speech::Actor {
            actor: target.to_string(),
            text: one_line(text)?.to_string(),
        });
    }
    Ok(Speech::Table {
        text: one_line(rest)?.to_string(),
    })
}

/// One say, in characters. Four kibibytes still fits in an 8 KiB player line.
const SAY_TEXT_CAP: usize = 4 * 1024;

fn one_line(text: &str) -> Result<&str, Reason> {
    if text.is_empty() || text.contains('\n') || text.chars().count() > SAY_TEXT_CAP {
        return Err(Reason::Schema);
    }
    Ok(text)
}

fn recipients(
    world: &World,
    speaker: Option<&str>,
    address: &str,
    place: Option<&str>,
    actor: Option<&str>,
) -> Vec<String> {
    let mut ids = BTreeSet::new();
    match address {
        "table" => {
            for id in seated_players(world) {
                ids.insert(id);
            }
        }
        "place" => {
            if let Some(place) = place {
                for id in seated_players(world) {
                    if world
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
                if seated_players(world).iter().any(|id| id == actor) {
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

fn seated_players(world: &World) -> Vec<String> {
    world
        .now()
        .presence
        .actors_present
        .iter()
        .filter(|id| world.now().actors.get(*id).is_some_and(ActorLive::is_pc))
        .cloned()
        .collect()
}
