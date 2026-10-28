//! Reading the `steps_run` field of a stored round record.

use serde_json::Value;

pub(crate) use crate::support::read_round;

/// Steps stored on `round`, or none when the field is left out. A rule step is its id.
pub(crate) fn read_steps_run(round: &Value) -> Vec<String> {
    round
        .get("steps_run")
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
