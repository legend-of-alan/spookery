//! Who hears a say. The host writes the tape.

use std::collections::BTreeSet;

use crate::model::{ActorLive, Now};

/// Where a say is addressed.
pub enum Heard<'a> {
    Table,
    Place(&'a str),
    Actor(&'a str),
}

/// Seated players who hear this say. The speaker is included.
pub fn recipients(now: &Now, speaker: Option<&str>, heard: Heard<'_>) -> Vec<String> {
    let mut ids = BTreeSet::new();
    match heard {
        Heard::Table => {
            ids.extend(seated(now));
        }
        Heard::Place(place) => {
            for id in seated(now) {
                if now
                    .actors
                    .get(&id)
                    .is_some_and(|actor| actor.place() == place)
                {
                    ids.insert(id);
                }
            }
        }
        Heard::Actor(actor) => {
            if seated(now).iter().any(|id| id == actor) {
                ids.insert(actor.to_string());
            }
        }
    }
    if let Some(speaker) = speaker {
        ids.insert(speaker.to_string());
    }
    ids.into_iter().collect()
}

fn seated(now: &Now) -> Vec<String> {
    now.presence
        .actors_present
        .iter()
        .filter(|id| now.actors.get(*id).is_some_and(ActorLive::is_pc))
        .cloned()
        .collect()
}
