//! Reading one pack file: its schema tag, its typed YAML, and where a problem sits in it.

use serde_saphyr::Spanned;

use crate::diagnostic::{locate_nth, parse_file, tag_diagnostic, Diagnostic, Severity};
use crate::error::Reason;
use crate::rule::Spot;

pub(super) const PACK_SCHEMA: &str = "pack";
pub(super) const PLACE_SCHEMA: &str = "place";
pub(super) const CAST_SCHEMA: &str = "cast";
pub(super) const METERS_SCHEMA: &str = "meters";
pub(super) const VERBS_SCHEMA: &str = "verbs";
pub(super) const ITEM_SCHEMA: &str = "item";
pub(super) const STORY_SCHEMA: &str = "story";
pub(super) const CHECKS_SCHEMA: &str = "checks";
pub(super) const TICK_SCHEMA: &str = "tick";

/// One pack file as the loader sees it: its pack-relative name, its YAML text,
/// and the file line the YAML starts on (1 for Markdown frontmatter, 0 for YAML).
pub(super) struct Source<'a> {
    pub file: &'a str,
    pub text: &'a str,
    pub line_base: u32,
}

impl<'a> Source<'a> {
    /// A file or directory whose text the caller does not hold, such as a lint's.
    pub fn named(file: &'a str) -> Self {
        Self {
            file,
            text: "",
            line_base: 0,
        }
    }
}

/// Where a problem is.
pub(super) enum At<'a> {
    /// No line; the path alone says where.
    Nowhere,
    /// The first `key` in the text, moved down by `line_base`.
    Key(&'a str),
    /// `key` inside the `row`th list item that holds an `id:`, counted in the text as
    /// given, without `line_base`. In a Markdown file the file's own `id:` is row 0, so a
    /// fact points one row early (issue #328).
    Row { row: usize, key: &'a str },
    /// A line and column serde recorded, already counted from the file's first line.
    Spot(Option<Spot>),
}

/// One problem in one pack file. `path` is `""` when there is none.
pub(super) struct Problem<'a> {
    pub source: &'a Source<'a>,
    pub path: &'a str,
    pub at: At<'a>,
    pub problem: String,
    pub hint: &'a str,
}

impl Problem<'_> {
    pub fn schema(self) -> Diagnostic {
        self.coded(Reason::Schema)
    }

    pub fn warning(self) -> Diagnostic {
        self.finish(Severity::Warning)
    }

    pub fn coded(self, reason: Reason) -> Diagnostic {
        self.finish(Severity::Error(reason))
    }

    fn finish(self, code: Severity) -> Diagnostic {
        let source = self.source;
        let mut diag = Diagnostic::new(source.file, code, &self.problem, self.hint);
        if !self.path.is_empty() {
            diag = diag.with_path(self.path);
        }
        match self.at {
            At::Nowhere | At::Spot(None) => diag,
            At::Key(key) => with_spot(diag, locate_nth(source.text, key, 0), source.line_base),
            At::Row { row, key } => {
                with_spot(diag, locate_row_field(source.text, "id:", row, key), 0)
            }
            At::Spot(Some(spot)) => {
                diag.at_line(spot.line, Some(spot.column).filter(|column| *column > 0))
            }
        }
    }
}

/// A message from a converter that does not know its file yet.
#[derive(Debug)]
pub(super) struct Fault {
    pub message: String,
    pub spot: Option<Spot>,
}

impl From<String> for Fault {
    fn from(message: String) -> Self {
        Self {
            message,
            spot: None,
        }
    }
}

impl From<&str> for Fault {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

/// The line and column serde recorded for a value, or `None` when it has no line.
pub(super) fn serde_spot<T>(span: &Spanned<T>) -> Option<Spot> {
    let line = u32::try_from(span.referenced.line()).unwrap_or(0);
    let column = u32::try_from(span.referenced.column()).unwrap_or(0);
    (line > 0).then_some(Spot { line, column })
}

/// `field` inside the `row`th list item that contains `marker` (`name:` or `id:`).
fn locate_row_field(text: &str, marker: &str, row: usize, field: &str) -> Option<(u32, u32)> {
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

fn with_spot(mut diag: Diagnostic, spot: Option<(u32, u32)>, line_base: u32) -> Diagnostic {
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
