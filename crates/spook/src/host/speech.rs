//! Append a say to the tape and tell whoever heard it.

use serde_json::{Map, Value};
use spookery_kernel::api::store::tape::{record_line, TapeRecord};
use spookery_kernel::api::world::World;
use spookery_kernel::api::Reason;

use super::frames;

#[derive(Debug)]
pub struct Spoken {
    pub tape: String,
    pub player_event: String,
    pub gm_event: Vec<u8>,
    pub tool: Value,
    pub recipients: Vec<String>,
}

pub fn speak(world: &World, actor_id: &str, line: &str) -> Result<Spoken, Reason> {
    let stored = world.speak_player(actor_id, line)?;
    Ok(spoken_from(&stored))
}

/// GM `say`. `from` is the seat. A supplied `from` appends nothing.
pub fn speak_tool(world: &World, arguments: Option<Map<String, Value>>) -> Result<Spoken, Reason> {
    let call = crate::protocol::read_say_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let stored = world.speak_gm(&call.address, call.place, call.actor, &call.text)?;
    Ok(spoken_from(&stored))
}

pub fn player_scrollback(world: &World, actor_id: &str) -> Result<Vec<String>, Reason> {
    let mut out = Vec::new();
    for stored in world.player_scrollback(actor_id)? {
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
