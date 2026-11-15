//! Visits: the places an actor has stood, and the exits they saw there.

use crate::state::component::{Component, Mut};
use crate::state::entity;
use crate::Reason;

/// A visit: a place an actor has stood at the end of a round, under the place's id, with the
/// names of the exits they saw there then.
pub struct Visit;

impl Component for Visit {
    type Value = Vec<String>;
    const NAME: &'static str = "visit";
}

impl Mut<'_, entity::Actors, Visit> {
    /// The actor saw `exits` at `place`, for `map`. No key answers for it, so nothing is
    /// reported.
    pub fn set(&mut self, place: &str, exits: Vec<String>) -> Result<(), Reason> {
        self.check_exists()?;
        let entry = (self.id.clone(), place.to_string());
        let (visited, transaction) = self.store_mut();
        visited.set(transaction, entry, Some(exits), [])
    }
}
