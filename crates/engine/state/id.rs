//! Entity identity: each kind's id, any entity's id as rules and the journal spell it, and the
//! borrowed form the canon's lookups take.

use std::borrow::Borrow;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Declares `$name`, the id of one kind of entity: a string saved as itself, which an id of
/// another kind cannot stand in for.
macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                $name(id.to_string())
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                $name(id)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

string_id! {
    /// An actor's id, as the cast and the save key them.
    ActorId
}

string_id! {
    /// A place's id, as the pack's places key them.
    PlaceId
}

string_id! {
    /// A story's id, as the pack's stories key them.
    StoryId
}

/// Who holds a meter, a flag or an item count, as a rule's `who` names it and the journal
/// writes it: `world`, `{actor: id}`, `{place: id}`, `{story: id}`, or an exit's door as
/// `{exit: [place, exit]}`. An exit carries only its flag `open`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityId {
    Actor(ActorId),
    Place(PlaceId),
    Story(StoryId),
    Exit(PlaceId, String),
    World,
}

/// The entity as a rule's `who` names it: an actor id, `place:<id>`, `story:<id>`,
/// `exit:<place>.<exit>` or `world`.
impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityId::Actor(id) => write!(f, "{id}"),
            EntityId::Place(id) => write!(f, "place:{id}"),
            EntityId::Story(id) => write!(f, "story:{id}"),
            EntityId::Exit(place, exit) => write!(f, "exit:{place}.{exit}"),
            EntityId::World => write!(f, "world"),
        }
    }
}

/// An entity by reference, for the canon's lookups: no id is cloned to ask what an entity
/// carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityRef<'a> {
    Actor(&'a ActorId),
    Place(&'a PlaceId),
    Story(&'a StoryId),
    Exit(&'a PlaceId, &'a str),
    World,
}

impl EntityId {
    pub fn as_ref(&self) -> EntityRef<'_> {
        match self {
            EntityId::Actor(id) => EntityRef::Actor(id),
            EntityId::Place(id) => EntityRef::Place(id),
            EntityId::Story(id) => EntityRef::Story(id),
            EntityId::Exit(place, exit) => EntityRef::Exit(place, exit),
            EntityId::World => EntityRef::World,
        }
    }
}
