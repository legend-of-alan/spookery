//! The item counts actors and places hold.

use crate::model::tracked::{Edit, EntityStore};

/// Every item count on each actor, or on each place, by the item's id. An entity with none of an item
/// has no count for it: putting the first one in adds the count, and taking the last one out
/// takes it off.
pub type Items = EntityStore<u32, Shift>;

/// A count moves by this much, to a count above 0 the writer has checked. Its undo moves it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shift(pub(crate) i64);

impl Edit<u32> for Shift {
    fn apply(self, count: &mut u32) -> Option<Self> {
        if self.0 == 0 {
            return None;
        }
        *count = u32::try_from(i64::from(*count).checked_add(self.0)?).ok()?;
        Some(Shift(-self.0))
    }
}

/// `current` plus `delta`. Removing from a count of 0 is `MISSING_ITEM`. A shortfall from a
/// positive count is `OUT_OF_RANGE`. A positive `max` caps the result.
pub(crate) fn shift_count(
    current: u32,
    delta: i64,
    max: Option<i64>,
) -> Result<u32, crate::Reason> {
    let current = i64::from(current);
    let mut next = current
        .checked_add(delta)
        .ok_or(crate::Reason::OutOfRange)?;
    if next < 0 {
        if current == 0 {
            return Err(crate::Reason::MissingItem);
        }
        return Err(crate::Reason::OutOfRange);
    }
    if delta > 0 {
        if let Some(cap) = max {
            if cap < 0 {
                return Err(crate::Reason::OutOfRange);
            }
            if next > cap {
                next = cap;
            }
        }
    }
    u32::try_from(next).map_err(|_| crate::Reason::OutOfRange)
}
