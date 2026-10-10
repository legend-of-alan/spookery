//! One problem a loader or the hidden-fact lint can show.
//!
//! `code` is a [`Reason`] token, or `HIDDEN_FACT_IN_BODY` for the lint.
//! The struct is plain data so a later JSON printer can serialize it as it stands.

use serde::Serialize;
use serde_saphyr::Error as YamlError;

use crate::error::Reason;
use crate::format::find_schema;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// Pack-relative path, with `/` on every OS.
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// Key path such as `moves[2].audience`.
    pub path: Option<String>,
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn new(file: &str, code: &str, problem: &str, hint: &str) -> Self {
        Self {
            file: file.replace('\\', "/"),
            line: None,
            column: None,
            path: None,
            code: code.to_string(),
            message: format!("{problem}; {hint}"),
            hint: Some(hint.to_string()),
        }
    }

    pub fn at_line(mut self, line: u32, column: Option<u32>) -> Self {
        self.line = Some(line);
        self.column = column;
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// `file:line: message (CODE)`, or `file: path: message (CODE)` when there is no line.
    pub fn report_line(&self) -> String {
        let head = match (self.line, self.path.as_deref()) {
            (Some(line), _) => format!("{}:{line}", self.file),
            (None, Some(path)) => format!("{}: {path}", self.file),
            (None, None) => self.file.clone(),
        };
        format!("{head}: {} ({})", self.message, self.code)
    }
}

pub fn reason_of(diagnostics: Vec<Diagnostic>) -> Reason {
    first_reason(&diagnostics)
}

pub fn first_reason(diagnostics: &[Diagnostic]) -> Reason {
    diagnostics
        .iter()
        .find_map(|diag| Reason::from_token(&diag.code))
        .unwrap_or(Reason::Schema)
}

pub(crate) fn schema(file: &str, problem: &str, hint: &str) -> Diagnostic {
    Diagnostic::new(file, Reason::Schema.token(), problem, hint)
}

pub(crate) fn coded(file: &str, code: Reason, problem: &str, hint: &str) -> Diagnostic {
    Diagnostic::new(file, code.token(), problem, hint)
}

/// Line and column of the `nth` match of `needle` in `text`. Both are 1-based.
pub(crate) fn locate_nth(text: &str, needle: &str, nth: usize) -> Option<(u32, u32)> {
    if needle.is_empty() {
        return None;
    }
    let mut seen = 0usize;
    for (index, line) in text.lines().enumerate() {
        let Some(col) = line.find(needle) else {
            continue;
        };
        if seen == nth {
            return Some((index as u32 + 1, col as u32 + 1));
        }
        seen += 1;
    }
    None
}

/// `field` inside the `row`th list item that contains `marker` (`name:` or `id:`).
pub(crate) fn locate_row_field(
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

pub(crate) fn with_spot(
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

/// Wrong or missing `schema` tag. `None` means the tag is the one this file uses.
pub(crate) fn tag_diagnostic(
    text: &str,
    expected: &str,
    file: &str,
    line_base: u32,
) -> Option<Diagnostic> {
    let hint = format!("write 'schema: {expected}' with no version suffix");
    let Some(found) = find_schema(text) else {
        return Some(
            schema(file, "the schema tag is missing", &hint)
                .with_path("schema")
                .at_line(1 + line_base, Some(1)),
        );
    };
    if found.value == expected {
        return None;
    }
    let problem = if found.value.contains('/') {
        format!("schema is '{}', an old versioned tag", found.value)
    } else {
        format!("schema is '{}', which this file does not use", found.value)
    };
    Some(
        schema(file, &problem, &hint)
            .with_path("schema")
            .at_line(found.line + line_base, Some(1)),
    )
}

pub(crate) fn yaml_diagnostic(file: &str, err: &YamlError, line_base: u32) -> Diagnostic {
    let err = err.without_snippet();
    let spot = err.location().and_then(|loc| {
        let line = u32::try_from(loc.line()).ok()?;
        if line == 0 {
            return None;
        }
        let column = u32::try_from(loc.column())
            .ok()
            .filter(|column| *column > 0);
        Some((line + line_base, column))
    });
    let (path, problem, hint) = match err {
        YamlError::SerdeUnknownField { field, .. } => (
            Some(field.clone()),
            format!("unknown field '{field}'"),
            "remove it or spell a field this file allows".to_string(),
        ),
        YamlError::SerdeMissingField { field, .. } => (
            Some((*field).to_string()),
            format!("missing field '{field}'"),
            format!("add '{field}'"),
        ),
        YamlError::SerdeInvalidType {
            unexpected,
            expected,
            ..
        } => (
            None,
            format!("found {unexpected} where a {expected} belongs"),
            format!("rewrite the value as a {expected}"),
        ),
        YamlError::SerdeInvalidValue {
            unexpected,
            expected,
            ..
        } => (
            None,
            format!("value {unexpected} is not allowed"),
            format!("use {expected}"),
        ),
        YamlError::SerdeUnknownVariant { variant, .. } => (
            Some(variant.clone()),
            format!("unknown value '{variant}'"),
            "use one of the values this field allows".to_string(),
        ),
        YamlError::InvalidScalar { ty, .. } => (
            None,
            format!("this value is not a valid {ty}"),
            format!("rewrite it as a {ty}"),
        ),
        _ => (
            None,
            "this YAML could not be read".to_string(),
            "fix the syntax at this spot and try again".to_string(),
        ),
    };
    let mut diag = schema(file, &problem, &hint);
    if let Some(path) = path {
        diag = diag.with_path(path);
    }
    if let Some((line, column)) = spot {
        diag = diag.at_line(line, column);
    }
    diag
}

pub(crate) fn parse_file<T: serde::de::DeserializeOwned>(
    file: &str,
    text: &str,
    line_base: u32,
) -> Result<T, Vec<Diagnostic>> {
    serde_saphyr::from_str(text).map_err(|err| vec![yaml_diagnostic(file, &err, line_base)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diagnostic_serializes_and_prints_the_line_form() {
        let diag = schema(
            "rules/moves.yaml",
            "row 'grit' has no audience",
            "add 'audience: player' or 'audience: habit'",
        )
        .with_path("moves[2].audience")
        .at_line(14, Some(5));
        assert_eq!(
            diag.report_line(),
            "rules/moves.yaml:14: row 'grit' has no audience; add 'audience: player' or 'audience: habit' (SCHEMA)"
        );
        let json = serde_json::to_value(&diag).unwrap();
        assert_eq!(json["file"], "rules/moves.yaml");
        assert_eq!(json["line"], 14);
        assert_eq!(json["column"], 5);
        assert_eq!(json["path"], "moves[2].audience");
        assert_eq!(json["code"], "SCHEMA");
        assert!(json["hint"].as_str().unwrap().contains("audience: player"));
        assert_eq!(first_reason(&[diag]), Reason::Schema);
    }

    #[test]
    fn a_path_is_the_location_when_the_line_is_unknown() {
        let diag = coded(
            "state/now.yaml",
            Reason::PackRevMismatch,
            "pack_rev does not match the canon hash",
            "run spook migrate to store the current hash",
        )
        .with_path("pack_rev");
        assert_eq!(
            diag.report_line(),
            "state/now.yaml: pack_rev: pack_rev does not match the canon hash; run spook migrate to store the current hash (PACK_REV_MISMATCH)"
        );
    }

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
        assert_eq!(err.code, "SCHEMA");
        assert!(err.line.is_some(), "{err:?}");
        let spot = locate_row_field(&format!("schema: moves\n{text}"), "name:", 0, "audiance");
        assert_eq!(spot, Some((3, 1)));
        assert_eq!(locate_nth(text, "name:", 0), Some((1, 1)));
        let shifted = with_spot(err.clone(), spot, 1);
        assert_eq!(shifted.line, Some(4));
    }

    #[test]
    fn an_old_tag_names_the_schema_line() {
        let text = "\n# note\nschema: moves/v0\nmoves: []\n";
        let diag = tag_diagnostic(text, "moves", "rules/moves.yaml", 0).unwrap();
        assert_eq!(diag.line, Some(3));
        assert!(diag.message.contains("old versioned tag"));
        assert!(diag.message.contains("no version"));
        assert_eq!(diag.code, "SCHEMA");
    }
}
