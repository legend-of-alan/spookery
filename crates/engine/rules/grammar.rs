//! The words a pack line names its actor, its owner and its receiver with: `subject:`, `of:` and `to:`.

use crate::ecs::id::{ActorId, ExitId, PlaceId, StoryId};

/// The actor an arrive or leave watch names with `subject:`. Omitted, it is `any`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Subject {
    Any,
    Pc,
    Npc,
    Id(String),
}

/// The actor a slot that needs one names: `subject:`, `to:` on a modifier or a memory, and a
/// nested check's `by:`, the actor told that check's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Actor {
    Self_,
    Target,
    Id(ActorId),
}

impl Actor {
    /// Reads an actor word: `self`, `target` or a cast id. An owner that is not an actor, such
    /// as `world` or `here`, is refused.
    pub(crate) fn parse(word: &str) -> Result<Actor, String> {
        match Object::parse(word)? {
            Object::Actor(actor) => Ok(actor),
            _ => Err(format!(
                "'{word}' is not an actor; name self, target or a cast id"
            )),
        }
    }
}

/// A grammatical object a pack line names, including but not limited to a direct object, an
/// indirect object or the object of a preposition. The pack writes `world` for
/// `Object::Global`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Object {
    Actor(Actor),
    Here,
    Global,
    Place(PlaceId),
    Story(StoryId),
    /// An exit, by its place and its exit id: `exit:<place>.<exit>`.
    Exit(ExitId),
}

impl Object {
    /// Reads an owner word: `self`, `target`, `here`, `world`, `place:<id>`, `story:<id>`,
    /// `exit:<place>.<exit>` or a cast id. It refuses `subject`, an empty word and the selector
    /// words retired packs wrote as owners; the error is the refusal a pack author reads.
    pub(crate) fn parse(word: &str) -> Result<Object, String> {
        if let Some(place) = word.strip_prefix("place:") {
            if place.is_empty() {
                return Err("place: needs an id".to_string());
            }
            return Ok(Object::Place(place.into()));
        }
        if let Some(story) = word.strip_prefix("story:") {
            if story.is_empty() {
                return Err("story: needs an id".to_string());
            }
            return Ok(Object::Story(story.into()));
        }
        if let Some(exit) = word.strip_prefix("exit:") {
            return match exit.split_once('.') {
                Some((place, exit_id)) if !place.is_empty() && !exit_id.is_empty() => {
                    Ok(Object::Exit(ExitId {
                        place: place.into(),
                        exit: exit_id.to_string(),
                    }))
                }
                _ => Err("exit: needs <place>.<exit>".to_string()),
            };
        }
        match word {
            "self" => Ok(Object::Actor(Actor::Self_)),
            "target" => Ok(Object::Actor(Actor::Target)),
            "here" => Ok(Object::Here),
            "world" => Ok(Object::Global),
            "subject" => Err(
                "'subject' is not an owner; use self, or target for the move's target".to_string(),
            ),
            "" | "token" | "first_rate" | "rate" | "first_drift" | "toward_target"
            | "first_known" | "first_listener" | "keep" | "first_due_note" | "first_owned" => {
                Err(format!("'{word}' is not an owner"))
            }
            actor_id => Ok(Object::Actor(Actor::Id(actor_id.into()))),
        }
    }
}
