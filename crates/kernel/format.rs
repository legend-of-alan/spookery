//! Format tags. One const per file. No version until the format is frozen.

pub const PACK: &str = "pack";
pub const PLACE: &str = "place";
pub const CAST: &str = "cast";
pub const METERS: &str = "meters";
pub const MOVES: &str = "moves";
pub const CHECKS: &str = "checks";
pub const TICK: &str = "tick";
pub const NOW: &str = "now";

pub(crate) struct FoundSchema<'a> {
    pub line: u32,
    pub value: &'a str,
}

/// The first `schema` key, with its 1-based line. Comments and blank lines are skipped.
pub(crate) fn find_schema(text: &str) -> Option<FoundSchema<'_>> {
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let value = unquote(trimmed.strip_prefix("schema:")?.trim());
        if value.is_empty() {
            return None;
        }
        return Some(FoundSchema {
            line: index as u32 + 1,
            value,
        });
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
