//! Item counts, and the rule for shifting one, the same in every collection that holds items.

use crate::state::change::{self, Change};
use crate::state::component::{Component, Mut, WieldedItem};
use crate::state::{entity, nested_map};
use crate::Reason;

/// An item: how many of it an entity holds. Actors and places hold items.
pub struct Item;

impl Component for Item {
    type Value = u32;
}

impl<E: entity::Has<Item>> Mut<'_, E, Item> {
    /// Move the count of `name` as `shift_count` says, reporting the change under `key`. A count
    /// of 0 is no entry.
    fn shift_with_key(
        &mut self,
        name: &str,
        delta: i64,
        max: Option<i64>,
        key: change::Key,
    ) -> Result<(), Reason> {
        self.check_exists()?;
        let current = self.writer.components.get(self.id, name).copied();
        let current = current.unwrap_or(0);
        let next = shift_count(current, delta, max)?;
        let count = |count: u32| change::Value::Number(count.into());
        let change = Change::new(key, count(current), count(next));
        let (tx, next) = (&mut *self.writer.transaction, (next > 0).then_some(next));
        (self.writer.components).set(tx, self.id, name, next, [change.into()])
    }
}

impl Mut<'_, entity::Actors, Item> {
    /// Move the actor's count of `name` as `shift_count` says. An actor who no longer holds the
    /// item they wield lets go of it.
    pub fn shift(&mut self, name: &str, delta: i64, max: Option<i64>) -> Result<(), Reason> {
        let key = change::Key::ActorItem(self.id.to_string(), name.to_string());
        self.shift_with_key(name, delta, max, key)?;
        let (actors, actor) = (&self.writer, self.id);
        let wielded = actors.wielded.get(actor);
        if wielded.is_some_and(|held| nested_map::get(&actors.items, actor, held).is_none()) {
            (self.writer.get_mut::<WieldedItem>(self.id)).set(None)?;
        }
        Ok(())
    }
}

impl Mut<'_, entity::Places, Item> {
    /// Move the count of `name` lying in the place as `shift_count` says. A place the pack does
    /// not define is `UNKNOWN_ID`.
    pub fn shift(&mut self, name: &str, delta: i64, max: Option<i64>) -> Result<(), Reason> {
        let key = change::Key::PlaceItem(self.id.to_string(), name.to_string());
        self.shift_with_key(name, delta, max, key)
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
