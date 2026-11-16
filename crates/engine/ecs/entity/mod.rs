//! The entities: each collection's stores and the components it registers, any entity's id as
//! rules and the journal spell it, and the borrowed form the canon's lookups take.

mod actors;
pub mod component;
mod globals;
mod places;
pub(in crate::ecs) mod query;
mod stories;

use std::fmt::{self, Debug};

use serde::{Deserialize, Serialize};

pub use actors::Actors;
pub use globals::Globals;
pub use places::Places;
pub use query::{Query, QueryData, QueryFilter, With, Without};
pub use stories::Stories;

use crate::canon::Canon;
use crate::ecs::id::{self, ActorId, PlaceId, StoryId};
use crate::ecs::Save;

/// A collection of entities: the actors, the places, the stories, or the one world. Its id type
/// is its own, so an id of another collection does not compile.
pub trait Collection {
    type Id: Ord + Clone + Debug + id::AsIdRef;

    /// Whether `id` is one of this collection's entities. The save lists the actors; the pack's
    /// `canon` defines the places and the stories; the world always is.
    fn contains(&self, canon: &Canon, id: &Self::Id) -> bool;

    /// Every one of this collection's entities, in id order, as `contains` lists them.
    fn ids<'a>(&'a self, canon: &'a Canon) -> query::Ids<'a, Self>;
}

/// How a collection keeps each component's store. Its module is private, so only the
/// collections, queries and component views under `entity` can name `Has`; code anywhere else
/// reads and writes through `Reader::get` and `Writer::get_mut`.
mod storage {
    use crate::ecs::component::{Component, Map};
    use crate::ecs::{entity::Collection, undo};

    /// A collection that keeps component `C` in its store `Store`.
    pub trait Has<C: Component>: Collection {
        type Store;
        fn store(&self) -> &Self::Store;
        fn store_mut(&mut self) -> &mut Self::Store;
    }

    /// The store of a component an entity holds many of, one under each entry id.
    pub type MapStore<E, C> = undo::Store<Map<<E as Collection>::Id, C>>;
}
use storage::{Has, MapStore};

/// Declares that `$entities` keeps `$component` in its field `$store`: by entry id, or, given the
/// field's type, at most one per entity.
macro_rules! has {
    ($entities:ty, $component:ty, $store:ident) => {
        entity::has!(@store $entities, $component, $store: entity::MapStore<$entities, $component>);
    };
    ($entities:ty, $component:ty, $store:ident: $type:ty) => {
        entity::has!(@store $entities, $component, $store: $type);

        impl entity::QueryData<$entities> for &$component {
            type Item<'a> = &'a <$component as component::Component>::Value;

            fn carriers<'a>(entities: Reader<'a, $entities>) -> Option<entity::query::Ids<'a, $entities>> {
                Some(Box::new(entities.components.$store.keys()))
            }

            fn fetch<'a>(
                entities: Reader<'a, $entities>,
                id: &'a <$entities as entity::Collection>::Id,
            ) -> Option<Self::Item<'a>> {
                entities.components.$store.get(id)
            }
        }
    };
    (@store $entities:ty, $component:ty, $store:ident: $type:ty) => {
        impl entity::Has<$component> for $entities {
            type Store = $type;
            fn store(&self) -> &$type {
                &self.$store
            }
            fn store_mut(&mut self) -> &mut $type {
                &mut self.$store
            }
        }
    };
}
use has;

/// `$body`, with `$typed` the id in the collection `$id` names and `$entities` that collection,
/// the field of the save `$save` (`&save` or `&mut save`). The one place an `entity::Id` picks
/// its collection, except a meter's value, whose actor arm adds modifiers.
macro_rules! in_collection {
    ($id:expr, [$($save:tt)+], |$entities:ident, $typed:ident| $body:expr) => {
        match $id {
            $crate::ecs::entity::Id::Actor($typed) => { let $entities = $($save)+.actors; $body },
            $crate::ecs::entity::Id::Place($typed) => { let $entities = $($save)+.places; $body },
            $crate::ecs::entity::Id::Story($typed) => { let $entities = $($save)+.stories; $body },
            $crate::ecs::entity::Id::Global => {
                let ($entities, $typed) = ($($save)+.globals, &$crate::ecs::id::GlobalId);
                $body
            }
        }
    };
}
pub(in crate::ecs) use in_collection;

/// The save, as the collection of every entity: the `entity::Id` picks the collection. Only a
/// component every collection holds has reads and writes on it, so an item does not compile.
impl Collection for Save {
    type Id = Id;

    fn contains(&self, canon: &Canon, id: &Id) -> bool {
        in_collection!(id, [&self], |entities, id| entities.contains(canon, id))
    }

    /// None: an `entity::Id` is built from a collection's id, not stored, so no query walks it.
    fn ids<'a>(&'a self, _: &'a Canon) -> query::Ids<'a, Self> {
        Box::new(std::iter::empty())
    }
}

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

impl id::AsIdRef for Id {
    fn as_id_ref(&self) -> IdRef<'_> {
        self.as_ref()
    }
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
