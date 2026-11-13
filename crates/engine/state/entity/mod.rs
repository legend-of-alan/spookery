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
use crate::state::id::{self, ActorId, GlobalId, PlaceId, StoryId};
use crate::state::transaction::Transaction;
use crate::state::{change, undo, Save};
use crate::Reason;

/// A collection of entities: the actors, the places, the stories, or the one world. Its id type
/// is its own, so an id of another collection does not compile.
pub trait Entities {
    type Id: Ord + Clone + Debug + id::AsIdRef;

    /// Whether `id` is one of this collection's entities. The save lists the actors; the pack's
    /// `canon` defines the places and the stories; the world always is.
    fn contains(&self, canon: &Canon, id: &Self::Id) -> bool;
}

/// A collection that keeps component `C` in a store of its own, under each entity's names.
pub trait Stores<C: Component>: Has<C> {
    fn store(&self) -> &undo::Store<component::Map<Self::Id, C>>;
    fn store_mut(&mut self) -> &mut undo::Store<component::Map<Self::Id, C>>;
}

/// A collection whose entities hold component `C`, read and written by id and name: each
/// collection `entity::has!` registers, and the save, which sends each id to its collection.
pub trait Has<C: Component>: Entities {
    /// What `id` holds under `name`.
    fn get(&self, id: &Self::Id, name: &str) -> Option<&C::Value>;

    /// Put `value` on `id` under `name`, `None` taking it off, as `undo::Store::set` does.
    fn set(
        &mut self,
        transaction: &mut Transaction,
        id: &Self::Id,
        name: &str,
        value: Option<C::Value>,
        reports: impl IntoIterator<Item = change::Report>,
    ) -> Result<(), Reason>;
}

/// The save, as the collection of every entity: each read and write goes to the collection the
/// id names. This is the one place a runtime `entity::Id` picks its collection.
impl Entities for Save {
    type Id = Id;

    fn contains(&self, canon: &Canon, id: &Id) -> bool {
        match id {
            Id::Actor(actor) => self.actors.contains(canon, actor),
            Id::Place(place) => self.places.contains(canon, place),
            Id::Story(story) => self.stories.contains(canon, story),
            Id::Global => self.world.contains(canon, &GlobalId),
        }
    }
}

/// A component every collection holds, on any entity.
impl<C: Component> Has<C> for Save
where
    Actors: Has<C>,
    Places: Has<C>,
    Stories: Has<C>,
    Globals: Has<C>,
{
    fn get(&self, id: &Id, name: &str) -> Option<&C::Value> {
        match id {
            Id::Actor(actor) => Has::<C>::get(&self.actors, actor, name),
            Id::Place(place) => Has::<C>::get(&self.places, place, name),
            Id::Story(story) => Has::<C>::get(&self.stories, story, name),
            Id::Global => Has::<C>::get(&self.world, &GlobalId, name),
        }
    }

    fn set(
        &mut self,
        transaction: &mut Transaction,
        id: &Id,
        name: &str,
        value: Option<C::Value>,
        reports: impl IntoIterator<Item = change::Report>,
    ) -> Result<(), Reason> {
        let tx = transaction;
        match id {
            Id::Actor(actor) => Has::<C>::set(&mut self.actors, tx, actor, name, value, reports),
            Id::Place(place) => Has::<C>::set(&mut self.places, tx, place, name, value, reports),
            Id::Story(story) => Has::<C>::set(&mut self.stories, tx, story, name, value, reports),
            Id::Global => Has::<C>::set(&mut self.world, tx, &GlobalId, name, value, reports),
        }
    }
}

/// Declares that `$entities` stores `$component` in its field `$store`.
macro_rules! has {
    ($entities:ty, $component:ty, $store:ident) => {
        const _: () = {
            use $crate::state::change::Report;
            use $crate::state::component::{Component, Map};
            use $crate::state::transaction::Transaction;
            use $crate::state::{nested_map, undo::Store};
            type Value = <$component as Component>::Value;

            impl $crate::state::entity::Stores<$component> for $entities {
                fn store(&self) -> &Store<Map<Self::Id, $component>> {
                    &self.$store
                }
                fn store_mut(&mut self) -> &mut Store<Map<Self::Id, $component>> {
                    &mut self.$store
                }
            }

            impl $crate::state::entity::Has<$component> for $entities {
                fn get(&self, id: &Self::Id, name: &str) -> Option<&Value> {
                    nested_map::get(&self.$store, id, name)
                }
                fn set(
                    &mut self,
                    transaction: &mut Transaction,
                    id: &Self::Id,
                    name: &str,
                    value: Option<Value>,
                    reports: impl IntoIterator<Item = Report>,
                ) -> Result<(), $crate::Reason> {
                    let entry = (id.clone(), name.to_string());
                    self.$store.set(transaction, entry, value, reports)
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
