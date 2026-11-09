//! One entity, resolved from the id a rule named, to read or to write. This module is outside
//! `state` on purpose: it sees only `State`'s public getters and handles, so every method here
//! can only select a kind and call that kind's method of the same name.

use crate::canon::Canon;
use crate::state::id::{ActorId, EntityId, PlaceId, StoryId};
use crate::state::{ActorComponents, PlaceComponents, State, StoryComponents, WorldComponents};
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
