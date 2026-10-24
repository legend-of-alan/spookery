//! Reading a stored tick record.

use std::path::Path;

use serde_json::Value;

pub(crate) fn read_tick(pack: &Path, key: &str) -> Value {
    let text = spookery_engine::testing::journal::read_record_line(pack, key)
        .unwrap_or_else(|err| panic!("{key}: {}", err.detail));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{key}: {err}\n{text}"))
}
