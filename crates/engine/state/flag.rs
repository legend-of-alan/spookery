//! The kind-free rule for setting a flag.

use std::collections::BTreeSet;

use super::change;
use super::transaction::Transaction;
use crate::Reason;

/// Put `entry` in `flags` when `on`, take it out when not, and report the change under `key`.
pub(super) fn set<K: Ord>(
    flags: &mut BTreeSet<K>,
    transaction: &mut Transaction,
    entry: K,
    on: bool,
    key: change::Key,
) -> Result<(), Reason> {
    let moved = if on {
        flags.insert(entry)
    } else {
        flags.remove(&entry)
    };
    if !moved {
        return Ok(());
    }
    transaction.report(key, change::Value::Bool(!on), change::Value::Bool(on))
}
