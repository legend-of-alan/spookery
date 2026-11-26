//! The words a pack line names its actor, its entity and its receiver with: `subject:`, `of:` and
//! `to:`, and who they name for an intent.

use crate::ecs::id::{ActorId, PathId, PlaceId, StoryId};
use crate::entity;
use crate::intent::Target;
use crate::rules::context::Context;
use crate::Reason;

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
    /// Reads an actor word: `self`, `target` or a cast id. An entity word that is not an actor, such
    /// as `world` or `here`, is refused.
    pub(crate) fn parse(word: &str) -> Result<Actor, String> {
        match Object::parse(word)? {
            Object::Actor(actor) => Ok(actor),
            _ => Err(format!(
                "'{word}' is not an actor; name self, target or a cast id"
            )),
        }
    }

    /// The actor this word names for `context`'s intent: the one acting, the move's target, or
    /// an actor by id. A target that is a place, a path or an item names no actor, so it is
    /// `UNKNOWN_ID`.
    pub(crate) fn actor_id<'a>(&'a self, context: &Context<'_, 'a>) -> Result<&'a ActorId, Reason> {
        match self {
            Actor::Self_ => context.intent.actor_id.as_ref().ok_or(Reason::Schema),
            Actor::Target => match &context.intent.target {
                Target::Actor(id) => Ok(id),
                Target::Empty => Err(Reason::Schema),
                Target::Place(_) | Target::Path(_) | Target::Item(_) => Err(Reason::UnknownId),
            },
            Actor::Id(id) => Ok(id),
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
    /// A path, `path:<id>`, whose one flag is `open`.
    Path(PathId),
    Story(StoryId),
}

impl Object {
    /// Reads an entity word: `self`, `target`, `here`, `world`, `place:<id>`, `path:<id>`,
    /// `story:<id>` or a cast id. It refuses `subject`, an empty word and the selector
    /// words retired packs wrote as entities; the error is the refusal a pack author reads.
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
        if let Some(path) = word.strip_prefix("path:") {
            if path.is_empty() {
                return Err("path: needs an id".to_string());
            }
            return Ok(Object::Path(path.into()));
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

    /// Who holds the meter, flag or item this entity word names for `context`'s intent: `here` and
    /// `place:` are a place, `path:` is a path, `story:` is a story, `world` is the world,
    /// `target` on a move whose target is a path is that path, and anyone else is an actor.
    pub(crate) fn entity_id(&self, context: &Context) -> Result<entity::Id, Reason> {
        match self {
            Object::Global => Ok(entity::Id::Global),
            Object::Story(id) => Ok(entity::Id::Story(id.clone())),
            Object::Path(id) => Ok(entity::Id::Path(id.clone())),
            Object::Place(id) => Ok(entity::Id::Place(id.clone())),
            Object::Here => Ok(entity::Id::Place(context.actor_place_id()?.into())),
            Object::Actor(Actor::Target) => match &context.intent.target {
                Target::Path(id) => Ok(entity::Id::Path(id.clone())),
                _ => Ok(entity::Id::Actor(Actor::Target.actor_id(context)?.clone())),
            },
            Object::Actor(actor) => Ok(entity::Id::Actor(actor.actor_id(context)?.clone())),
        }
    }
}
