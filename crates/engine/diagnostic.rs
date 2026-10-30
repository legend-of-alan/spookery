//! One problem a loader or pack check can show.
//!
//! `code` is an [`Error`](Code::Error) [`Reason`] or a [`Warning`](Code::Warning).
//! The struct is plain data so a later JSON printer can serialize it as it stands.

use std::fmt;

use serde::{Serialize, Serializer};
use serde_saphyr::Error as YamlError;

use crate::error::Reason;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Error(Reason),
    Warning,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Error(reason) => reason.as_str(),
            Code::Warning => "WARNING",
        }
    }
}

impl Serialize for Code {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

pub(crate) const NOW_SCHEMA: &str = "now";

struct FoundSchema<'a> {
    line: u32,
    value: &'a str,
}

/// The first `schema` key, with its 1-based line. Comments and blank lines are skipped.
fn find_schema(text: &str) -> Option<FoundSchema<'_>> {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// Pack-relative path, with `/` on every OS.
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// Key path such as `rules[0].when`.
    pub path: Option<String>,
    pub code: Code,
    pub message: String,
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn new(file: &str, code: Code, problem: &str, hint: &str) -> Self {
        Self {
            file: file.replace('\\', "/"),
            line: None,
            column: None,
            path: None,
            code,
            message: format!("{problem}; {hint}"),
            hint: Some(hint.to_string()),
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self.code, Code::Error(_))
    }

    pub(crate) fn at_line(mut self, line: u32, column: Option<u32>) -> Self {
        self.line = Some(line);
        self.column = column;
        self
    }

    /// At `spot`, a line and column `line_base` lines into the file. No spot leaves it as is.
    pub(crate) fn at_spot(self, spot: Option<(u32, u32)>, line_base: u32) -> Self {
        match spot {
            Some((line, column)) => self.at_line(line + line_base, Some(column)),
            None => self,
        }
    }

    pub(crate) fn at_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// `file:line: message (CODE)`, or `file: path: message (CODE)` when there is no line.
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.path.as_deref()) {
            (Some(line), _) => write!(f, "{}:{line}", self.file)?,
            (None, Some(path)) => write!(f, "{}: {path}", self.file)?,
            (None, None) => f.write_str(&self.file)?,
        }
        write!(f, ": {} ({})", self.message, self.code.as_str())
    }
}

pub fn first_reason(diagnostics: &[Diagnostic]) -> Reason {
    diagnostics
        .iter()
        .find_map(|diag| match diag.code {
            Code::Error(reason) => Some(reason),
            Code::Warning => None,
        })
        .unwrap_or(Reason::Schema)
}

pub(crate) fn schema_error(file: &str, problem: &str, hint: &str) -> Diagnostic {
    error(file, Reason::Schema, problem, hint)
}

pub(crate) fn error(file: &str, code: Reason, problem: &str, hint: &str) -> Diagnostic {
    Diagnostic::new(file, Code::Error(code), problem, hint)
}

pub(crate) fn warning(file: &str, problem: &str, hint: &str) -> Diagnostic {
    Diagnostic::new(file, Code::Warning, problem, hint)
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

/// Wrong or missing `schema` tag. `None` means the tag is the one this file uses.
pub(crate) fn check_tag(
    text: &str,
    expected: &str,
    file: &str,
    line_base: u32,
) -> Option<Diagnostic> {
    let hint = format!("write 'schema: {expected}' with no version suffix");
    let Some(found) = find_schema(text) else {
        return Some(
            schema_error(file, "the schema tag is missing", &hint)
                .at_path("schema")
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
        schema_error(file, &problem, &hint)
            .at_path("schema")
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
    let mut diag = schema_error(file, &problem, &hint);
    if let Some(path) = path {
        diag = diag.at_path(path);
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
        let diag = schema_error(
            "rules/verbs.yaml",
            "rule 'grit' is missing when",
            "add when",
        )
        .at_path("rules[0].when")
        .at_line(14, Some(5));
        assert_eq!(
            diag.to_string(),
            "rules/verbs.yaml:14: rule 'grit' is missing when; add when (SCHEMA)"
        );
        let json = serde_json::to_value(&diag).unwrap();
        assert_eq!(json["file"], "rules/verbs.yaml");
        assert_eq!(json["line"], 14);
        assert_eq!(json["column"], 5);
        assert_eq!(json["path"], "rules[0].when");
        assert_eq!(json["code"], "SCHEMA");
        assert!(json["hint"].as_str().unwrap().contains("add when"));
        assert_eq!(first_reason(&[diag]), Reason::Schema);
    }

    #[test]
    fn a_path_is_the_location_when_the_line_is_unknown() {
        let diag = error(
            "state/save.yaml",
            Reason::PackRevMismatch,
            "pack_rev does not match the canon hash",
            "run spook migrate to store the current hash",
        )
        .at_path("pack_rev");
        assert_eq!(
            diag.to_string(),
            "state/save.yaml: pack_rev: pack_rev does not match the canon hash; run spook migrate to store the current hash (PACK_REV_MISMATCH)"
        );
    }

    #[test]
    fn an_old_tag_names_the_schema_line() {
        let text = "\n# note\nschema: moves/v0\nmoves: []\n";
        let diag = check_tag(text, "moves", "rules/moves.yaml", 0).unwrap();
        assert_eq!(diag.line, Some(3));
        assert!(diag.message.contains("old versioned tag"));
        assert!(diag.message.contains("no version"));
        assert_eq!(diag.code, Code::Error(Reason::Schema));
    }
}
