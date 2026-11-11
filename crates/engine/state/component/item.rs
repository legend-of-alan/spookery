//! Item counts, and the rule for shifting one, the same in every collection.

use crate::state::change::{self, Change};
use crate::Reason;

/// `current` moved by `delta`. Removing from a count of 0 is `MISSING_ITEM`, and a shortfall from
/// a positive count is `OUT_OF_RANGE`. A positive `max` caps the result.
pub(super) fn shift_count(current: u32, delta: i64, max: Option<i64>) -> Result<u32, Reason> {
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

/// The count `held` moved as `shift_count` says, and the change that reports under `key`. A count
/// of 0 is no entry.
pub(in crate::state) fn shift(
    held: Option<u32>,
    delta: i64,
    max: Option<i64>,
    key: change::Key,
) -> Result<(Option<u32>, Change), Reason> {
    let current = held.unwrap_or(0);
    let next = shift_count(current, delta, max)?;
    let count = |count: u32| change::Value::Number(count.into());
    let change = Change::new(key, count(current), count(next));
    Ok(((next > 0).then_some(next), change))
}
