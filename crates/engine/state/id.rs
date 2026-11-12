//! The id of an actor, of a place, of a story and of the world, and the address of an exit.

use std::borrow::Borrow;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::state::entity;

/// Declares `$name`, the id of an entity in one collection: a string saved as itself, which an id
/// from another collection cannot stand in for.
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

/// The world's id. There is one world, so its id says nothing more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlobalId;

/// An id of one collection, in the borrowed form the canon's lookups take.
pub trait AsIdRef {
    fn as_id_ref(&self) -> entity::IdRef<'_>;
}

impl AsIdRef for ActorId {
    fn as_id_ref(&self) -> entity::IdRef<'_> {
        entity::IdRef::Actor(self)
    }
}

impl AsIdRef for PlaceId {
    fn as_id_ref(&self) -> entity::IdRef<'_> {
        entity::IdRef::Place(self)
    }
}

impl AsIdRef for StoryId {
    fn as_id_ref(&self) -> entity::IdRef<'_> {
        entity::IdRef::Story(self)
    }
}

impl AsIdRef for GlobalId {
    fn as_id_ref(&self) -> entity::IdRef<'_> {
        entity::IdRef::Global
    }
}

/// One exit's door, by its place and its name. An address, not an entity: it holds nothing but
/// whether the door is open.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExitId {
    pub place: PlaceId,
    pub exit: String,
}

/// Where a walker on this exit stands: `exit:<place>.<exit>`.
impl fmt::Display for ExitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "exit:{}.{}", self.place, self.exit)
    }
}
