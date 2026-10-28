//! Reading one pack file: its schema tag, its typed YAML, and where a problem sits in it.

use crate::diagnostic::{parse_file, tag_diagnostic, Diagnostic};

pub(super) const PACK_SCHEMA: &str = "pack";
pub(super) const PLACE_SCHEMA: &str = "place";
pub(super) const CAST_SCHEMA: &str = "cast";
pub(super) const METERS_SCHEMA: &str = "meters";
pub(super) const VERBS_SCHEMA: &str = "verbs";
pub(super) const ITEM_SCHEMA: &str = "item";
pub(super) const STORY_SCHEMA: &str = "story";
pub(super) const CHECKS_SCHEMA: &str = "checks";
pub(super) const TICK_SCHEMA: &str = "tick";

/// `field` inside the `row`th list item that contains `marker` (`name:` or `id:`).
pub(super) fn locate_row_field(
    text: &str,
    marker: &str,
    row: usize,
    field: &str,
) -> Option<(u32, u32)> {
    let mut marks = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.contains(marker) {
            marks.push(index);
        }
    }
    let start = *marks.get(row)?;
    let end = marks.get(row + 1).copied().unwrap_or(usize::MAX);
    for (index, line) in text.lines().enumerate().skip(start) {
        if index >= end {
            break;
        }
        if let Some(col) = line.find(field) {
            return Some((index as u32 + 1, col as u32 + 1));
        }
    }
    None
}

pub(super) fn with_spot(
    mut diag: Diagnostic,
    spot: Option<(u32, u32)>,
    line_base: u32,
) -> Diagnostic {
    if let Some((line, column)) = spot {
        diag.line = Some(line + line_base);
        diag.column = Some(column);
    }
    diag
}

pub(super) fn load_headed<T: serde::de::DeserializeOwned>(
    file: &str,
    text: &str,
    expected: &str,
    line_base: u32,
) -> Result<T, Vec<Diagnostic>> {
    if let Some(diag) = tag_diagnostic(text, expected, file, line_base) {
        return Err(vec![diag]);
    }
    parse_file(file, text, line_base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::{locate_nth, Severity};
    use crate::error::Reason;

    #[test]
    fn a_misspelled_field_keeps_the_yaml_line() {
        #[derive(Debug, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Row {
            name: String,
        }
        let ok: Row = parse_file("rules/moves.yaml", "name: grit\n", 0).unwrap();
        assert_eq!(ok.name, "grit");
        let text = "name: grit\naudiance: player\n";
        let err = parse_file::<Row>("rules/moves.yaml", text, 0).unwrap_err();
        let err = &err[0];
        assert!(err.message.contains("unknown field 'audiance'"), "{err:?}");
        assert_eq!(err.code, Severity::Error(Reason::Schema));
        assert!(err.line.is_some(), "{err:?}");
        let spot = locate_row_field(&format!("schema: moves\n{text}"), "name:", 0, "audiance");
        assert_eq!(spot, Some((3, 1)));
        assert_eq!(locate_nth(text, "name:", 0), Some((1, 1)));
        let shifted = with_spot(err.clone(), spot, 1);
        assert_eq!(shifted.line, Some(4));
    }
}
