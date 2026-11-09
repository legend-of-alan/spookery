//! One entity, resolved from the id a rule named, to read or to write. This module is outside
//! `state` on purpose: it sees only `State`'s public getters and handles, so every method here
//! can only select a kind and call that kind's method of the same name.

use crate::canon::Canon;
use crate::state::id::{ActorId, EntityId, PlaceId, StoryId};
use crate::state::meter::Meter;
use crate::state::{
    ActorComponents, PlaceComponents, State, StoryComponents, WorldComponents, Writer,
};
use crate::Reason;

/// One entity's components, to read: the kind's components and the id. It borrows the save
/// immutably, so a condition reads through it with no transaction in hand.
pub enum Entity<'a> {
    Actor(&'a ActorComponents, &'a ActorId),
    Place(&'a PlaceComponents, &'a PlaceId),
    Story(&'a StoryComponents, &'a StoryId),
    World(&'a WorldComponents),
}

impl State {
    /// The entity `id` names, to read.
    pub fn entity<'a>(&'a self, id: &'a EntityId) -> Entity<'a> {
        match id {
            EntityId::Actor(actor) => Entity::Actor(self.actors(), actor),
            EntityId::Place(place) => Entity::Place(self.places(), place),
            EntityId::Story(story) => Entity::Story(self.stories(), story),
            EntityId::World => Entity::World(self.world()),
        }
    }
}

impl Entity<'_> {
    /// The current value of meter `key`, or `UNKNOWN_ID` when the entity does not carry it.
    pub fn meter_value(&self, canon: &Canon, key: &str) -> Result<i64, Reason> {
        match self {
            Entity::Actor(actors, id) => actors.meter_value(canon, id, key),
            Entity::Place(places, id) => places.meter_value(canon, id, key),
            Entity::Story(stories, id) => stories.meter_value(canon, id, key),
            Entity::World(world) => world.meter_value(canon, key),
        }
    }

    pub fn has_flag(&self, key: &str) -> bool {
        match self {
            Entity::Actor(actors, id) => actors.has_flag(id, key),
            Entity::Place(places, id) => places.has_flag(id, key),
            Entity::Story(stories, id) => stories.has_flag(id, key),
            Entity::World(world) => world.has_flag(key),
        }
    }
}

/// The same entity, to write: the kind's `Writer` and the id. Every method selects the variant
/// and calls the kind writer's method of the same name; nothing here validates, writes, counts
/// or reports on its own, and nothing here could.
pub enum EntityMut<'a> {
    Actor(Writer<'a, ActorComponents>, &'a ActorId),
    Place(Writer<'a, PlaceComponents>, &'a PlaceId),
    Story(Writer<'a, StoryComponents>, &'a StoryId),
    World(Writer<'a, WorldComponents>),
}

impl State {
    /// The entity `id` names, to write.
    pub fn entity_mut<'a>(&'a mut self, id: &'a EntityId) -> EntityMut<'a> {
        match id {
            EntityId::Actor(actor) => EntityMut::Actor(self.actors_mut(), actor),
            EntityId::Place(place) => EntityMut::Place(self.places_mut(), place),
            EntityId::Story(story) => EntityMut::Story(self.stories_mut(), story),
            EntityId::World => EntityMut::World(self.world_mut()),
        }
    }
}

impl EntityMut<'_> {
    pub fn shift_meter(&mut self, canon: &Canon, key: &str, delta: i64) -> Result<(), Reason> {
        match self {
            EntityMut::Actor(actors, id) => actors.shift_meter(canon, id, key, delta),
            EntityMut::Place(places, id) => places.shift_meter(canon, id, key, delta),
            EntityMut::Story(stories, id) => stories.shift_meter(canon, id, key, delta),
            EntityMut::World(world) => world.shift_meter(canon, key, delta),
        }
    }

    pub fn set_flag(&mut self, key: &str, on: bool) -> Result<(), Reason> {
        match self {
            EntityMut::Actor(actors, id) => actors.set_flag(id, key, on),
            EntityMut::Place(places, id) => places.set_flag(id, key, on),
            EntityMut::Story(stories, id) => stories.set_flag(id, key, on),
            EntityMut::World(world) => world.set_flag(key, on),
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
            EntityMut::Actor(actors, actor) => actors.shift_item(actor, id, delta, max),
            EntityMut::Place(places, place) => places.shift_item(canon, place, id, delta, max),
            EntityMut::Story(..) | EntityMut::World(_) => Err(Reason::Schema),
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
            EntityMut::Actor(actors, id) => actors.set_meter(canon, id, key, meter),
            EntityMut::Place(places, id) => places.set_meter(canon, id, key, meter),
            EntityMut::Story(stories, id) => stories.set_meter(canon, id, key, meter),
            EntityMut::World(world) => world.set_meter(canon, key, meter),
        }
    }
}
