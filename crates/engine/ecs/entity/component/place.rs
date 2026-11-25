//! Where each actor stands.

use crate::ecs::change::Change;
use crate::ecs::component::ActorLocation;
use crate::ecs::component::{text_value, Component, Mut, Ref};
use crate::ecs::entity;
use crate::ecs::id::PlaceId;
use crate::Reason;

/// Where an actor stands.
pub struct Place;

impl Component for Place {
    type Value = ActorLocation;
    const ID: &'static str = "place";
}

impl<'a> Ref<'a, entity::Actors, Place> {
    /// Where the actor stands.
    pub fn get(&self) -> Option<&'a ActorLocation> {
        self.store().get(self.id)
    }

    /// The place the actor stands in, `None` while they walk.
    pub fn place_id(&self) -> Option<&'a PlaceId> {
        self.get().and_then(ActorLocation::place_id)
    }
}

impl Mut<'_, entity::Actors, Place> {
    /// Stand the actor at `to`: in a place, or on a path while they walk.
    pub fn set(&mut self, to: impl Into<ActorLocation>) -> Result<(), Reason> {
        self.check_exists()?;
        let to = to.into();
        let key = self.key(None);
        let before = text_value(self.store().get(self.id).map(ActorLocation::to_string));
        let change = Change::new(key, before, text_value(Some(to.to_string())));
        let (id, (place, transaction)) = (self.id, self.store_mut());
        place.set(transaction, id, to, [change.into()])
    }
}
