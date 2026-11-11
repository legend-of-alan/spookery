//! The entities: each collection's stores, any entity's id as rules and the journal spell it,
//! and the borrowed form the canon's lookups take.

pub(in crate::state) mod actors;
mod globals;
mod places;
mod stories;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use actors::Actors;
pub use globals::Globals;
pub use places::Places;
pub use stories::Stories;

use crate::state::id::{ActorId, PlaceId, StoryId};

/// Who holds a meter, a flag or an item count, as a rule's `of` names it and the journal
/// writes it: `world`, `{actor: id}`, `{place: id}` or `{story: id}`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Id {
    Actor(ActorId),
    Place(PlaceId),
    Story(StoryId),
    #[serde(rename = "world")]
    Global,
}

/// The entity as a rule's `of` names it: an actor id, `place:<id>`, `story:<id>` or `world`.
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Id::Actor(id) => write!(f, "{id}"),
            Id::Place(id) => write!(f, "place:{id}"),
            Id::Story(id) => write!(f, "story:{id}"),
            Id::Global => write!(f, "world"),
        }
    }
}

/// An entity by reference, for the canon's lookups: no id is cloned to ask what an entity
/// carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdRef<'a> {
    Actor(&'a ActorId),
    Place(&'a PlaceId),
    Story(&'a StoryId),
    Global,
}

impl Id {
    pub fn as_ref(&self) -> IdRef<'_> {
        match self {
            Id::Actor(id) => IdRef::Actor(id),
            Id::Place(id) => IdRef::Place(id),
            Id::Story(id) => IdRef::Story(id),
            Id::Global => IdRef::Global,
        }
    }
}
