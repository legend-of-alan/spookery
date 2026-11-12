//! One entity, resolved from the id a rule named, to read or to write. This module is outside
//! `state` on purpose: it sees only `State`'s public getters and handles, so every method here
//! can only select a collection and read or write the same component on it.

use std::marker::PhantomData;

use crate::state::component::{Component, Flag, Meter};
pub use crate::state::entity::{Id, IdRef};
use crate::state::id::{ActorId, GlobalId, PlaceId, StoryId};
use crate::state::{self, Reader, State, Writer};
use crate::Reason;

/// One entity, to read: the collection's `Reader` and the id. It borrows the save immutably, so
/// a condition reads through it with no transaction in hand.
#[derive(Clone, Copy)]
pub enum Ref<'a> {
    Actor(Reader<'a, state::entity::Actors>, &'a ActorId),
    Place(Reader<'a, state::entity::Places>, &'a PlaceId),
    Story(Reader<'a, state::entity::Stories>, &'a StoryId),
    Global(Reader<'a, state::entity::Globals>, &'a GlobalId),
}

impl State {
    /// The entity `id` names, to read.
    pub fn entity<'a>(&'a self, id: &'a Id) -> Ref<'a> {
        match id {
            Id::Actor(actor) => Ref::Actor(self.actors(), actor),
            Id::Place(place) => Ref::Place(self.places(), place),
            Id::Story(story) => Ref::Story(self.stories(), story),
            Id::Global => Ref::Global(self.globals(), &GlobalId),
        }
    }
}

/// The same entity, to write: the collection's `Writer` and the id. Nothing here validates,
/// writes, counts or reports on its own; the typed handle does.
pub enum Mut<'a> {
    Actor(Writer<'a, state::entity::Actors>, &'a ActorId),
    Place(Writer<'a, state::entity::Places>, &'a PlaceId),
    Story(Writer<'a, state::entity::Stories>, &'a StoryId),
    Global(Writer<'a, state::entity::Globals>, &'a GlobalId),
}

impl State {
    /// The entity `id` names, to write.
    pub fn entity_mut<'a>(&'a mut self, id: &'a Id) -> Mut<'a> {
        match id {
            Id::Actor(actor) => Mut::Actor(self.actors_mut(), actor),
            Id::Place(place) => Mut::Place(self.places_mut(), place),
            Id::Story(story) => Mut::Story(self.stories_mut(), story),
            Id::Global => Mut::Global(self.globals_mut(), &GlobalId),
        }
    }
}

/// `$body`, with `$entities` the `Reader` or `Writer` of the collection the `entity::Ref` or
/// `entity::Mut` `$entity` selects and `$id` the entity's id there: the one place a component
/// read or write picks the collection.
macro_rules! in_collection {
    ($entity:expr, $enum:ident($entities:ident, $id:ident) => $body:expr) => {
        match $entity {
            $enum::Actor($entities, $id) => $body,
            $enum::Place($entities, $id) => $body,
            $enum::Story($entities, $id) => $body,
            $enum::Global($entities, $id) => $body,
        }
    };
}

/// One entity's component `C`, to read with the typed handle's methods.
pub struct ComponentRef<'a, C>(Ref<'a>, PhantomData<C>);

impl<'a> Ref<'a> {
    /// The entity's component `C`, read with the typed handle's methods. Only a component every
    /// collection holds has them, so reading one that some collection lacks does not compile.
    pub fn get<C: Component>(self) -> ComponentRef<'a, C> {
        ComponentRef(self, PhantomData)
    }
}

impl ComponentRef<'_, Meter> {
    /// As `component::Ref::value`: an actor's adds their modifiers.
    pub fn value(&self, name: &str) -> Result<i64, Reason> {
        in_collection!(self.0, Ref(entities, id) => entities.get::<Meter>(id).value(name))
    }
}

impl ComponentRef<'_, Flag> {
    /// As `component::Ref::contains`.
    pub fn contains(&self, name: &str) -> bool {
        in_collection!(self.0, Ref(entities, id) => entities.get::<Flag>(id).contains(name))
    }
}

/// One entity's component `C`, to write with the typed handle's methods.
pub struct ComponentMut<'m, 'a, C>(&'m mut Mut<'a>, PhantomData<C>);

impl<'a> Mut<'a> {
    /// The entity's component `C`, written with the typed handle's methods. Only a component
    /// every collection holds has them, so writing one that some collection lacks does not
    /// compile: an item goes through `state.actors_mut()` or `state.places_mut()`.
    pub fn get_mut<C: Component>(&mut self) -> ComponentMut<'_, 'a, C> {
        ComponentMut(self, PhantomData)
    }
}

impl ComponentMut<'_, '_, Meter> {
    /// As `component::Mut::shift`.
    pub fn shift(&mut self, name: &str, delta: i64) -> Result<(), Reason> {
        in_collection!(self.0, Mut(entities, id) => {
            entities.get_mut::<Meter>(id).shift(name, delta)
        })
    }

    /// As `component::Mut::set`: migrate and fixtures.
    pub fn set(&mut self, name: &str, meter: Option<Meter>) -> Result<(), Reason> {
        in_collection!(self.0, Mut(entities, id) => entities.get_mut::<Meter>(id).set(name, meter))
    }
}

impl ComponentMut<'_, '_, Flag> {
    /// As `component::Mut::set`.
    pub fn set(&mut self, name: &str, on: bool) -> Result<(), Reason> {
        in_collection!(self.0, Mut(entities, id) => entities.get_mut::<Flag>(id).set(name, on))
    }
}
