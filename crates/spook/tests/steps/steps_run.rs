//! Reading the `steps_run` field of a stored tick record.

use std::path::Path;

use serde_json::Value;

/// Steps every round pushes. The journal leaves them out when the round
/// followed the standard order.
const STANDARD_STEPS: &[&str] = &[
    "advance",
    "complete",
    "gather",
    "session_status",
    "resolve",
    "apply",
    "run_triggers",
    "observe",
    "record",
    "commit_round",
];

pub(crate) fn read_tick(pack: &Path, key: &str) -> Value {
    let text = spookery_kernel::api::journal::read_record_line(pack, key)
        .unwrap_or_else(|err| panic!("{key}: {}", err.detail));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{key}: {err}\n{text}"))
}

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

pub(crate) fn assert_standard_steps_omitted(tick: &Value) {
    let names = read_steps_run(tick);
    for step in STANDARD_STEPS {
        assert!(
            !names.iter().any(|name| name == step),
            "{step} is in steps_run\n{tick}"
        );
    }
}
