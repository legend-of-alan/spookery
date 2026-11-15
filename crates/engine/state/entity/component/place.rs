//! Where each actor stands.

use crate::state::change::Change;
use crate::state::component::{text_value, Component, Mut, Ref};
use crate::state::entity;
use crate::Reason;

/// Where an actor stands: a place id, or `exit:<place>.<exit>` while they walk.
pub struct Place;

impl Component for Place {
    type Value = String;
    const NAME: &'static str = "place";
}

impl<'a> Ref<'a, entity::Actors, Place> {
    /// Where the actor stands.
    pub fn get(&self) -> Option<&'a str> {
        self.store().get(self.id).map(String::as_str)
    }
}

impl Mut<'_, entity::Actors, Place> {
    /// Stand the actor at `to`: a place id, or an exit address while they walk.
    pub fn set(&mut self, to: &str) -> Result<(), Reason> {
        self.check_exists()?;
        let key = self.key(None);
        let before = text_value(self.store().get(self.id));
        let change = Change::new(key, before, text_value(Some(to)));
        let (id, (place, transaction)) = (self.id, self.store_mut());
        place.set(transaction, id, to.to_string(), [change.into()])
    }
}
