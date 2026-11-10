//! The kind-free rule for a flag's next entry.

use super::change::{self, Change};

/// A flag's entry when `on`, none when not, and the change that reports under `key`.
pub(super) fn next(on: bool, key: change::Key) -> (Option<()>, Change) {
    let change = Change::new(key, change::Value::Bool(!on), change::Value::Bool(on));
    (on.then_some(()), change)
}
