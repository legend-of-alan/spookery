//! One entity, resolved from the id a rule named, to read or to write. This module is outside
//! `state` on purpose: it sees only `State`'s public getters and handles, so every method here
//! can only select a collection and call that collection's method of the same name.

use crate::canon::Canon;
use crate::state::component::meter::Meter;
pub use crate::state::entity::{Id, IdRef};
use crate::state::id::{ActorId, PlaceId, StoryId};
use crate::state::{self, State, Writer};
use crate::Reason;

/// One entity's components, to read: the collection's components and the id. It borrows the save
/// immutably, so a condition reads through it with no transaction in hand.
pub enum Ref<'a> {
    Actor(&'a state::entity::Actors, &'a ActorId),
    Place(&'a state::entity::Places, &'a PlaceId),
    Story(&'a state::entity::Stories, &'a StoryId),
    Global(&'a state::entity::Globals),
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
    /// The current value of meter `key`, or `UNKNOWN_ID` when the entity does not carry it.
    pub fn meter_value(&self, canon: &Canon, key: &str) -> Result<i64, Reason> {
        match self {
            Ref::Actor(actors, id) => actors.meter_value(canon, id, key),
            Ref::Place(places, id) => places.meter_value(canon, id, key),
            Ref::Story(stories, id) => stories.meter_value(canon, id, key),
            Ref::Global(globals) => globals.meter_value(canon, key),
        }
    }

    pub fn has_flag(&self, key: &str) -> bool {
        match self {
            Ref::Actor(actors, id) => actors.has_flag(id, key),
            Ref::Place(places, id) => places.has_flag(id, key),
            Ref::Story(stories, id) => stories.has_flag(id, key),
            Ref::Global(globals) => globals.has_flag(key),
        }
    }
}

/// The same entity, to write: the collection's `Writer` and the id. Every method selects the variant
/// and calls the collection writer's method of the same name; nothing here validates, writes, counts
/// or reports on its own, and nothing here could.
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
    pub fn shift_meter(&mut self, canon: &Canon, key: &str, delta: i64) -> Result<(), Reason> {
        match self {
            Mut::Actor(actors, id) => actors.shift_meter(canon, id, key, delta),
            Mut::Place(places, id) => places.shift_meter(canon, id, key, delta),
            Mut::Story(stories, id) => stories.shift_meter(canon, id, key, delta),
            Mut::Global(globals) => globals.shift_meter(canon, key, delta),
        }
    }

    pub fn set_flag(&mut self, key: &str, on: bool) -> Result<(), Reason> {
        match self {
            Mut::Actor(actors, id) => actors.set_flag(id, key, on),
            Mut::Place(places, id) => places.set_flag(id, key, on),
            Mut::Story(stories, id) => stories.set_flag(id, key, on),
            Mut::Global(globals) => globals.set_flag(key, on),
        }
    }

    /// Actors and places hold items; a story or the world is `SCHEMA`, decided here once.
    pub fn shift_item(
        &mut self,
        canon: &Canon,
        id: &str,
        delta: i64,
        max: Option<i64>,
    ) -> Result<(), Reason> {
        match self {
            Mut::Actor(actors, actor) => actors.shift_item(actor, id, delta, max),
            Mut::Place(places, place) => places.shift_item(canon, place, id, delta, max),
            Mut::Story(..) | Mut::Global(_) => Err(Reason::Schema),
        }
    }

    /// Give the entity meter `key`, `None` taking it off, outside a round's rules: migrate.
    pub fn set_meter(
        &mut self,
        canon: &Canon,
        key: &str,
        meter: Option<Meter>,
    ) -> Result<(), Reason> {
        match self {
            Mut::Actor(actors, id) => actors.set_meter(canon, id, key, meter),
            Mut::Place(places, id) => places.set_meter(canon, id, key, meter),
            Mut::Story(stories, id) => stories.set_meter(canon, id, key, meter),
            Mut::Global(globals) => globals.set_meter(canon, key, meter),
        }
    }
}
