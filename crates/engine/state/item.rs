//! Item counts, and the kind-free rule for shifting one.

use std::collections::BTreeMap;
use std::fmt::Debug;

use super::change::{self, Change};
use super::transaction::Transaction;
use super::undo::Store;
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

/// Move the count under `entry` as `shift_count` says, and report the change under `key`. A count
/// of 0 is no entry.
pub(super) fn shift<K: Ord + Clone + Debug>(
    counts: &mut Store<BTreeMap<K, u32>>,
    transaction: &mut Transaction,
    entry: K,
    delta: i64,
    max: Option<i64>,
    key: change::Key,
) -> Result<(), Reason> {
    let current = counts.get(&entry).copied().unwrap_or(0);
    let next = shift_count(current, delta, max)?;
    let count = |count: u32| change::Value::Number(count.into());
    let change = Change::new(key, count(current), count(next));
    counts.set(transaction, entry, (next > 0).then_some(next), [change])
}
