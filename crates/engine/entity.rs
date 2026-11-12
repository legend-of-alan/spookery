//! One entity, resolved from the id a rule named, to read or to write. This module is outside
//! `state` on purpose: it sees only `State`'s public getters and handles, so every method here
//! can only select a collection and read or write the same component on it.

use crate::state::component::{self, modifier, Meter};
pub use crate::state::entity::{Id, IdRef};
use crate::state::id::{ActorId, GlobalId, PlaceId, StoryId};
use crate::state::{self, Reader, State, Writer};
use crate::Reason;

/// One entity's components, to read: the collection's `Reader` and the id. It borrows the save
/// immutably, so a condition reads through it with no transaction in hand.
pub enum Ref<'a> {
    Actor(Reader<'a, state::entity::Actors>, &'a ActorId),
    Place(Reader<'a, state::entity::Places>, &'a PlaceId),
    Story(Reader<'a, state::entity::Stories>, &'a StoryId),
    Global(Reader<'a, state::entity::Globals>),
}

impl State {
    /// The entity `id` names, to read.
    pub fn entity<'a>(&'a self, id: &'a Id) -> Ref<'a> {
        match id {
            Id::Actor(actor) => Ref::Actor(self.actors(), actor),
            Id::Place(place) => Ref::Place(self.places(), place),
            Id::Story(story) => Ref::Story(self.stories(), story),
            Id::Global => Ref::Global(self.globals()),
        }
    }
}

impl Ref<'_> {
    /// The current value of meter `key`, an actor's with their modifiers, or `UNKNOWN_ID` when
    /// the entity does not carry it.
    pub fn meter_value(&self, key: &str) -> Result<i64, Reason> {
        match *self {
            Ref::Actor(actors, id) => modifier::value(actors, id, key),
            Ref::Place(places, id) => places.get::<Meter>(id).value(key),
            Ref::Story(stories, id) => stories.get::<Meter>(id).value(key),
            Ref::Global(globals) => globals.get::<Meter>(&GlobalId).value(key),
        }
    }

    pub fn has_flag(&self, key: &str) -> bool {
        match *self {
            Ref::Actor(actors, id) => actors.get::<component::Flag>(id).contains(key),
            Ref::Place(places, id) => places.get::<component::Flag>(id).contains(key),
            Ref::Story(stories, id) => stories.get::<component::Flag>(id).contains(key),
            Ref::Global(globals) => globals.get::<component::Flag>(&GlobalId).contains(key),
        }
    }
}

/// The same entity, to write: the collection's `Writer` and the id. Every method selects the variant
/// and makes the same component write on it; nothing here validates, writes, counts or reports on
/// its own, and nothing here could.
pub enum Mut<'a> {
    Actor(Writer<'a, state::entity::Actors>, &'a ActorId),
    Place(Writer<'a, state::entity::Places>, &'a PlaceId),
    Story(Writer<'a, state::entity::Stories>, &'a StoryId),
    Global(Writer<'a, state::entity::Globals>),
}

impl State {
    /// The entity `id` names, to write.
    pub fn entity_mut<'a>(&'a mut self, id: &'a Id) -> Mut<'a> {
        match id {
            Id::Actor(actor) => Mut::Actor(self.actors_mut(), actor),
            Id::Place(place) => Mut::Place(self.places_mut(), place),
            Id::Story(story) => Mut::Story(self.stories_mut(), story),
            Id::Global => Mut::Global(self.globals_mut()),
        }
    }
}

impl Mut<'_> {
    pub fn shift_meter(&mut self, key: &str, delta: i64) -> Result<(), Reason> {
        match self {
            Mut::Actor(actors, id) => actors.get_mut::<Meter>(id).shift(key, delta),
            Mut::Place(places, id) => places.get_mut::<Meter>(id).shift(key, delta),
            Mut::Story(stories, id) => stories.get_mut::<Meter>(id).shift(key, delta),
            Mut::Global(globals) => globals.get_mut::<Meter>(&GlobalId).shift(key, delta),
        }
    }

    pub fn set_flag(&mut self, key: &str, on: bool) -> Result<(), Reason> {
        match self {
            Mut::Actor(actors, id) => actors.get_mut::<component::Flag>(id).set(key, on),
            Mut::Place(places, id) => places.get_mut::<component::Flag>(id).set(key, on),
            Mut::Story(stories, id) => stories.get_mut::<component::Flag>(id).set(key, on),
            Mut::Global(globals) => globals.get_mut::<component::Flag>(&GlobalId).set(key, on),
        }
    }

    /// Actors and places hold items; a story or the world is `SCHEMA`, decided here once.
    pub fn shift_item(&mut self, id: &str, delta: i64, max: Option<i64>) -> Result<(), Reason> {
        match self {
            Mut::Actor(actors, actor) => actors
                .get_mut::<component::Item>(actor)
                .shift(id, delta, max),
            Mut::Place(places, place) => places
                .get_mut::<component::Item>(place)
                .shift(id, delta, max),
            Mut::Story(..) | Mut::Global(_) => Err(Reason::Schema),
        }
    }

    /// Give the entity meter `key`, `None` taking it off, outside a round's rules: migrate.
    pub fn set_meter(&mut self, key: &str, meter: Option<Meter>) -> Result<(), Reason> {
        match self {
            Mut::Actor(actors, id) => actors.get_mut::<Meter>(id).set(key, meter),
            Mut::Place(places, id) => places.get_mut::<Meter>(id).set(key, meter),
            Mut::Story(stories, id) => stories.get_mut::<Meter>(id).set(key, meter),
            Mut::Global(globals) => globals.get_mut::<Meter>(&GlobalId).set(key, meter),
        }
    }
}
