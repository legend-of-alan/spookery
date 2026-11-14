//! Where each actor stands.

use crate::state::change::{self, Change};
use crate::state::component::{text_value, Component, Mut, Ref};
use crate::state::entity;
use crate::Reason;

/// Where an actor stands: a place id, or `exit:<place>.<exit>` while they walk.
pub struct Place;

impl Component for Place {
    type Value = String;
}

impl<'a> Ref<'a, entity::Actors, Place> {
    /// Where the actor stands.
    pub fn get(&self) -> Option<&'a str> {
        (self.entities.components.place.get(self.id)).map(String::as_str)
    }
}

impl Mut<'_, entity::Actors, Place> {
    /// Stand the actor at `to`: a place id, or an exit address while they walk.
    pub fn set(&mut self, to: &str) -> Result<(), Reason> {
        self.check_exists()?;
        let key = change::Key::ActorPlace(self.id.to_string());
        let before = text_value(self.writer.components.place.get(self.id));
        let change = Change::new(key, before, text_value(Some(to)));
        let (place, to) = (&mut self.writer.components.place, to.to_string());
        place.set(self.writer.transaction, self.id, to, [change.into()])
    }
}
