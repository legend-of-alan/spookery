//! Format tags. One const per file. No version until the format is frozen.

use crate::error::Reason;

pub const PACK: &str = "pack";
pub const PLACE: &str = "place";
pub const CAST: &str = "cast";
pub const METERS: &str = "meters";
pub const MOVES: &str = "moves";
pub const CHECKS: &str = "checks";
pub const TICK: &str = "tick";
pub const NOW: &str = "now";

/// `SCHEMA` unless the first YAML key is `expected`. The rest of the text is not parsed.
pub fn require_tag(text: &str, expected: &str) -> Result<(), Reason> {
    if schema_tag(text) == Some(expected) {
        Ok(())
    } else {
        Err(Reason::Schema)
    }
}

fn schema_tag(text: &str) -> Option<&str> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let value = unquote(line.strip_prefix("schema:")?.trim());
        if value.is_empty() {
            return None;
        }
        return Some(value);
    }
    None
}

fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let quote = bytes[0];
        if (quote == b'"' || quote == b'\'') && bytes[bytes.len() - 1] == quote {
            return &value[1..value.len() - 1];
        }
    }
    value
}
