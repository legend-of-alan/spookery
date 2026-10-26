//! Reading the `steps_run` field of a stored tick record.

use serde_json::Value;

pub(crate) use crate::support::read_tick;

/// Step names stored on `tick`, or none when the field is left out.
pub(crate) fn read_steps_run(tick: &Value) -> Vec<String> {
    tick.get("steps_run")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}
