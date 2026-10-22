//! Append a say to the tape and tell whoever heard it.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use spookery_kernel::api::model::ActorLive;
use spookery_kernel::api::speech::Heard;
use spookery_kernel::api::store::tape::{
    append_record, read_session, read_tail, record_line, TapeRecord,
};
use spookery_kernel::api::world::World;
use spookery_kernel::api::Reason;

use super::frames;
use super::input::{split_token, verb_line};

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
    spookery_kernel::api::store::tape::session_end_seq(root)
}

/// Stored records with `seq` greater than `since`, in file order, inside the tape window.
pub fn tape_after(root: &std::path::Path, since: u64) -> Result<Vec<TapeRecord>, Reason> {
    read_session(root, since, None)
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
    let stored = append_record(world.root(), |seq| TapeRecord {
        seq,
        from: from.to_string(),
        address: address.to_string(),
        place: place.clone(),
        actor: actor.clone(),
        recipients: recipients.clone(),
        text: text.clone(),
        tick_index,
        world_time,
    })?;
    Ok(spoken_from(&stored))
}

pub fn player_scrollback(root: &std::path::Path, actor_id: &str) -> Result<Vec<String>, Reason> {
    let mut out = Vec::new();
    for stored in read_tail(root, actor_id, 256)? {
        out.push(frames::player_replay_line(&stored));
    }
    Ok(out)
}

fn spoken_from(stored: &TapeRecord) -> Spoken {
    Spoken {
        tape: record_line(stored),
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
    let (verb, rest) = verb_line(line, seated)?;
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
    let heard = match address {
        "table" => Heard::Table,
        "place" => Heard::Place(place.unwrap_or("")),
        "actor" => Heard::Actor(actor.unwrap_or("")),
        _ => return Vec::new(),
    };
    spookery_kernel::api::speech::recipients(world.now(), speaker, heard)
}
