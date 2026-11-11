//! The rule for a flag's next entry, the same in every collection.

use crate::state::change::{self, Change};

/// A flag's entry when `on`, none when not, and the change that reports under `key`.
pub(in crate::state) fn next(on: bool, key: change::Key) -> (Option<()>, Change) {
    let change = Change::new(key, change::Value::Bool(!on), change::Value::Bool(on));
    (on.then_some(()), change)
}
