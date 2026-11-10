//! The kind-free rule for setting a flag.

use super::change::{self, Change};

/// A flag's entry when `on`, none when not, and the change that reports under `key`.
pub(super) fn set(on: bool, key: change::Key) -> (Option<()>, Change) {
    let change = Change::new(key, change::Value::Bool(!on), change::Value::Bool(on));
    (on.then_some(()), change)
}
