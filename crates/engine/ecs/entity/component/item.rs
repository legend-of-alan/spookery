//! Item counts, and the rule for shifting one, the same in every collection that holds items.

use crate::ecs::change::{Change, ChangeValue};
use crate::ecs::component::{Component, Mut, WieldedItem};
use crate::ecs::{entity, Reader};
use crate::Reason;

/// An item: how many of it an entity holds. Actors and places hold items.
pub struct Item;

impl Component for Item {
    type Value = u32;
    const ID: &'static str = "item";
}

impl<E: entity::Has<Item, Store = entity::MapStore<E, Item>>> Mut<'_, E, Item> {
    /// Move the count of `entry_id` the entity holds as `shift_count` says. A count of 0 is no
    /// entry.
    fn shift_held(&mut self, entry_id: &str, delta: i64, max: Option<i64>) -> Result<(), Reason> {
        self.check_exists()?;
        let entry = (self.id.clone(), entry_id.to_string());
        let current = self.store().get(&entry).copied().unwrap_or(0);
        let next = shift_count(current, delta, max)?;
        let count = |count: u32| Some(ChangeValue::Number(count.into()));
        let change = Change::new(self.key(Some(entry_id)), count(current), count(next));
        let (store, transaction) = self.store_mut();
        store.set(
            transaction,
            entry,
            (next > 0).then_some(next),
            [change.into()],
        )
    }
}

impl Mut<'_, entity::Actors, Item> {
    /// Move the actor's count of `entry_id` as `shift_count` says. An actor who no longer holds the
    /// item they wield lets go of it.
    pub fn shift(&mut self, entry_id: &str, delta: i64, max: Option<i64>) -> Result<(), Reason> {
        self.shift_held(entry_id, delta, max)?;
        let actors = Reader::new(&*self.writer.components, self.writer.canon);
        let wielded = actors.get::<WieldedItem>(self.id).get();
        if wielded.is_some_and(|item_id| !actors.get::<Item>(self.id).contains(item_id)) {
            (self.writer.get_mut::<WieldedItem>(self.id)).set(None)?;
        }
        Ok(())
    }
}

impl Mut<'_, entity::Places, Item> {
    /// Move the count of `entry_id` lying in the place as `shift_count` says. A place the pack does
    /// not define is `UNKNOWN_ID`.
    pub fn shift(&mut self, entry_id: &str, delta: i64, max: Option<i64>) -> Result<(), Reason> {
        self.shift_held(entry_id, delta, max)
    }
}

/// `current` moved by `delta`. Removing from a count of 0 is `MISSING_ITEM`, and a shortfall from
/// a positive count is `OUT_OF_RANGE`. A positive `max` caps the result.
fn shift_count(current: u32, delta: i64, max: Option<i64>) -> Result<u32, Reason> {
    let current = i64::from(current);
    let mut next = current.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if next < 0 {
        if current == 0 {
            return Err(Reason::MissingItem);
        }
        return Err(Reason::OutOfRange);
    }
    if delta > 0 {
        if let Some(cap) = max {
            if cap < 0 {
                return Err(Reason::OutOfRange);
            }
            next = next.min(cap);
        }
    }
    u32::try_from(next).map_err(|_| Reason::OutOfRange)
}
