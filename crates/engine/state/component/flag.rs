//! Flags: a name set on an entity, or not there.

use crate::state::change;
use crate::state::component::{Component, Mut};
use crate::state::entity;
use crate::state::prelude::*;
use crate::Reason;

/// A flag: set on an entity under its name, or not there.
pub struct Flag;

impl Component for Flag {
    type Value = ();
}

impl<E: entity::Has<Flag>> Mut<'_, E, Flag> {
    /// Set flag `name` on or off. An entity the save or the pack does not have is `UNKNOWN_ID`.
    pub fn set(&mut self, name: &str, on: bool) -> Result<(), Reason> {
        let key = change::Key::flag(self.id.as_id_ref(), name);
        self.set_with_key(name, on, key)
    }
}
