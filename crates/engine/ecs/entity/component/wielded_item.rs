//! The item each actor wields.

use crate::ecs::change::Change;
use crate::ecs::component::{text_value, Component, Item, Mut, Ref};
use crate::ecs::{entity, Reader};
use crate::Reason;

/// The item an actor wields, which they must hold.
pub struct WieldedItem;

impl Component for WieldedItem {
    type Value = String;
    const ID: &'static str = "wielded_item";
}

impl<'a> Ref<'a, entity::Actors, WieldedItem> {
    /// The item the actor wields.
    pub fn get(&self) -> Option<&'a str> {
        self.store().get(self.id).map(String::as_str)
    }
}

impl Mut<'_, entity::Actors, WieldedItem> {
    /// Wield `item`, which the actor must hold, or with `None` let go of what they wield.
    pub fn set(&mut self, item: Option<&str>) -> Result<(), Reason> {
        self.check_exists()?;
        let key = self.key(None);
        let before = text_value(self.store().get(self.id));
        let reports = [Change::new(key, before, text_value(item)).into()];
        let items = Reader::new(&*self.writer.components, self.writer.canon).get::<Item>(self.id);
        if item.is_some_and(|item| !items.contains(item)) {
            return Err(Reason::MissingItem);
        }
        let (id, (wielded, transaction)) = (self.id.clone(), self.store_mut());
        wielded.set(transaction, id, item.map(String::from), reports)
    }
}
