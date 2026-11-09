//! The kind-free rule for setting a flag.

use std::collections::BTreeSet;
use std::fmt::Debug;

use super::change::{self, Change};
use super::transaction::Transaction;
use super::undo::Store;
use crate::Reason;

/// Put `entry` in `flags` when `on`, take it out when not, and report the change under `key`.
pub(super) fn set<K: Ord + Clone + Debug>(
    flags: &mut Store<BTreeSet<K>>,
    transaction: &mut Transaction,
    entry: K,
    on: bool,
    key: change::Key,
) -> Result<(), Reason> {
    let change = Change::new(key, change::Value::Bool(!on), change::Value::Bool(on));
    flags.set(transaction, entry, on.then_some(()), [change])
}
