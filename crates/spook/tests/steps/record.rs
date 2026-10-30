//! Reading the `extra_steps` field of a stored round record.

use serde_json::Value;

pub(crate) use crate::support::read_round;

/// Steps stored on `round`, or none when the field is left out. A rule step is its id.
pub(crate) fn read_extra_steps(round: &Value) -> Vec<String> {
    round
        .get("extra_steps")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().or_else(|| value["rule"].as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
