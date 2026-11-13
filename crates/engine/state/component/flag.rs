//! Flags: a name set on an entity, or not there.

use crate::state::change;
use crate::state::component::{Component, Mut, Ref};
use crate::state::entity::{self, in_collection};
use crate::state::prelude::*;
use crate::state::{Reader, Save, Writer};
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

impl Ref<'_, Save, Flag> {
    /// As on its collection.
    pub fn contains(&self, name: &str) -> bool {
        let (save, canon) = (self.entities.components, self.entities.canon);
        in_collection!(self.id, [&save], |entities, id| {
            Reader::new(entities, canon).get::<Flag>(id).contains(name)
        })
    }
}

impl Mut<'_, Save, Flag> {
    /// As on its collection.
    pub fn set(&mut self, name: &str, on: bool) -> Result<(), Reason> {
        let (tx, canon) = (&mut *self.writer.transaction, self.writer.canon);
        in_collection!(self.id, [&mut self.writer.components], |entities, id| {
            Writer::new(entities, tx, canon)
                .get_mut::<Flag>(id)
                .set(name, on)
        })
    }
}
