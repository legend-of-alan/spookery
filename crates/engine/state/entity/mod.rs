//! The entities: each collection's stores and the components it registers, any entity's id as
//! rules and the journal spell it, and the borrowed form the canon's lookups take.

pub(in crate::state) mod actors;
mod globals;
mod places;
mod stories;

use std::fmt::{self, Debug};

use serde::{Deserialize, Serialize};

pub use actors::Actors;
pub use globals::Globals;
pub use places::Places;
pub use stories::Stories;

use crate::canon::Canon;
use crate::state::component::{self, Component};
use crate::state::id::{self, ActorId, PlaceId, StoryId};
use crate::state::undo;
use crate::Reason;

/// A collection of entities: the actors, the places, the stories, or the one world. Its id type
/// is its own, so an id of another collection does not compile.
pub trait Entities {
    type Id: Ord + Clone + Debug + id::AsIdRef;

    /// Whether `id` is one of this collection's entities. The save lists the actors; the pack's
    /// `canon` defines the places and the stories; the world always is.
    fn contains(&self, canon: &Canon, id: &Self::Id) -> bool;

    /// What `id`'s modifiers add to meter `key`. Only actors hold modifiers.
    fn modifier_sum(&self, _: &Self::Id, _: &str) -> Result<i64, Reason> {
        Ok(0)
    }
}

/// A collection that stores component `C` under each entity's names.
pub trait Has<C: Component>: Entities {
    fn store(&self) -> &undo::Store<component::Map<Self::Id, C>>;
    fn store_mut(&mut self) -> &mut undo::Store<component::Map<Self::Id, C>>;
}

/// Declares that `$entities` stores `$component` in its field `$store`.
macro_rules! has {
    ($entities:ty, $component:ty, $store:ident) => {
        const _: () = {
            use $crate::state::{component::Map, undo::Store};

            impl $crate::state::entity::Has<$component> for $entities {
                fn store(&self) -> &Store<Map<Self::Id, $component>> {
                    &self.$store
                }
                fn store_mut(&mut self) -> &mut Store<Map<Self::Id, $component>> {
                    &mut self.$store
                }
            }
        };
    };
}
pub(in crate::state) use has;

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
