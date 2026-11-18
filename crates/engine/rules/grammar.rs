//! The words a pack line names its actor, its owner and its receiver with: `subject:`, `of:` and `to:`.

use crate::ecs::id::{ActorId, ExitId, PlaceId, StoryId};

/// The actor an arrive or leave watch names with `subject:`. Omitted, it is `any`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    Any,
    Pc,
    Npc,
    Actor(String),
}

/// A grammatical object a pack line names, including but not limited to a direct object, an
/// indirect object or the object of a preposition. A resist's `subject:` (the actor who rolls
/// to resist, not the subject of the outer line) reuses this value set until pack lines have a
/// parse tree. The pack writes `world` for `Object::Global`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Object {
    Self_,
    Target,
    Here,
    Global,
    Place(PlaceId),
    Story(StoryId),
    /// An exit, by its place and its exit id: `exit:<place>.<exit>`.
    Exit(ExitId),
    Actor(ActorId),
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
                    let (place, exit) = (place.into(), exit_id.to_string());
                    Ok(Object::Exit(ExitId { place, exit }))
                }
                _ => Err("exit: needs <place>.<exit>".to_string()),
            };
        }
        match word {
            "self" => Ok(Object::Self_),
            "target" => Ok(Object::Target),
            "here" => Ok(Object::Here),
            "world" => Ok(Object::Global),
            "subject" => Err(
                "'subject' is not an owner; use self, or target for the move's target".to_string(),
            ),
            "" | "token" | "first_rate" | "rate" | "first_drift" | "toward_target"
            | "first_known" | "first_listener" | "keep" | "first_due_note" | "first_owned" => {
                Err(format!("'{word}' is not an owner"))
            }
            actor_id => Ok(Object::Actor(actor_id.into())),
        }
    }
}
