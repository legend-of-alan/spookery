//! The item each actor wields.

use crate::state::change::{self, Change};
use crate::state::component::{text_value, Component, Mut, Ref};
use crate::state::{entity, nested_map};
use crate::Reason;

/// The item an actor wields, which they must hold.
pub struct WieldedItem;

impl Component for WieldedItem {
    type Value = String;
}

impl<'a> Ref<'a, entity::Actors, WieldedItem> {
    /// The item the actor wields.
    pub fn get(&self) -> Option<&'a str> {
        (self.entities.components.wielded.get(self.id)).map(String::as_str)
    }
}

impl Mut<'_, entity::Actors, WieldedItem> {
    /// Wield `item`, which the actor must hold, or with `None` let go of what they wield.
    pub fn set(&mut self, item: Option<&str>) -> Result<(), Reason> {
        self.check_exists()?;
        let key = change::Key::ActorWielded(self.id.to_string());
        let before = text_value(self.writer.components.wielded.get(self.id));
        let reports = [Change::new(key, before, text_value(item)).into()];
        let items = &self.writer.components.items;
        if item.is_some_and(|item| nested_map::get(items, self.id, item).is_none()) {
            return Err(Reason::MissingItem);
        }
        let (wielded, item) = (&mut self.writer.components.wielded, item.map(String::from));
        wielded.set(self.writer.transaction, self.id.clone(), item, reports)
    }
}
