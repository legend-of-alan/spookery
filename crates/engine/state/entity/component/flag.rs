//! Flags: a name set on an entity, or not there.

use crate::state::component::{Component, Mut, Ref};
use crate::state::entity::in_collection;
use crate::state::{Reader, Save, Writer};
use crate::Reason;

/// A flag: set on an entity under its name, or not there.
pub struct Flag;

impl Component for Flag {
    type Value = ();
    const NAME: &'static str = "flag";
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
        let (transaction, canon) = (&mut *self.writer.transaction, self.writer.canon);
        in_collection!(self.id, [&mut self.writer.components], |entities, id| {
            Writer::new(entities, transaction, canon)
                .get_mut::<Flag>(id)
                .set(name, on)
        })
    }
}
