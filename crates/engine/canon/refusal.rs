//! Placing a refusal on a pack file and line.

use serde_saphyr::Spanned;

use crate::diagnostic::{locate_nth, Code, Diagnostic};
use crate::rules::rule::LineColumn;
use crate::Reason;

/// One pack file as the loader sees it: its pack-relative name and its YAML text, which
/// starts on the file's first line.
pub(super) struct FileText<'a> {
    pub file: &'a str,
    pub text: &'a str,
}

impl<'a> FileText<'a> {
    /// A file or directory whose text the caller does not hold, such as a lint's.
    pub(super) fn without_text(file: &'a str) -> Self {
        Self { file, text: "" }
    }

    /// The refusal at `key_path` when `id` is empty.
    pub(super) fn refuse_empty(
        &self,
        id: &str,
        key_path: &str,
        at: Locator,
        words: EmptyWords,
    ) -> Option<Diagnostic> {
        let (message, hint) = words;
        id.is_empty()
            .then(|| self.refuse_at(key_path, at, message.into(), hint))
    }

    /// The refusal at `key_path` for an `id` the caller found a second time.
    pub(super) fn refuse_duplicate(
        &self,
        id: &str,
        key_path: &str,
        at: Locator,
        words: DuplicateWords,
    ) -> Diagnostic {
        let (before, after, hint) = words;
        self.refuse_at(key_path, at, format!("{before} '{id}' {after}"), hint)
    }

    /// The refusals for the `index`th entry of the top-level `{noun}s:` list, whose id is
    /// `id`: an empty id, then each constraint the entry breaks, then, when nothing else is
    /// wrong and `is_duplicate` says an earlier entry has this id, the duplicate.
    pub(super) fn refuse_entry(
        &self,
        noun: &str,
        index: usize,
        id: &str,
        is_duplicate: bool,
        constraints: impl IntoIterator<Item = EntryConstraint>,
    ) -> Vec<Diagnostic> {
        let list = format!("{noun}s:");
        let refuse = |field: &str, message: String, hint: &str| {
            let key = format!(
                "{}:",
                field.rsplit_once('.').map_or(field, |(_, last)| last)
            );
            let at = Locator::Entry {
                list: &list,
                index,
                key: &key,
            };
            self.refuse_at(&format!("{noun}s[{index}].{field}"), at, message, hint)
        };
        let empty_message = format!("a {noun} id is empty");
        let broken = constraints.into_iter().filter(|(broken, ..)| *broken);
        let mut diagnostics: Vec<Diagnostic> = id
            .is_empty()
            .then(|| refuse("id", empty_message, &format!("set id to a short {noun} id")))
            .into_iter()
            .chain(broken.map(|(_, field, message, hint)| refuse(field, message, hint)))
            .collect();
        if diagnostics.is_empty() && is_duplicate {
            let duplicate_message = format!("{noun} '{id}' is listed twice");
            diagnostics.push(refuse("id", duplicate_message, &format!("keep one {noun}")));
        }
        diagnostics
    }

    /// A `CompileError` at the file location it recorded, refused as a schema error.
    pub(super) fn refuse_error(&self, key_path: &str, error: CompileError) -> Diagnostic {
        let at = Locator::FileLocation(error.file_location);
        self.refuse_at(key_path, at, error.message, "")
    }

    /// A schema error at `key_path` (such as `rules[0].when`), on the line and column `at` finds.
    pub(super) fn refuse_at(
        &self,
        key_path: &str,
        at: Locator,
        message: String,
        hint: &str,
    ) -> Diagnostic {
        let code = Code::Error(Reason::Schema);
        let mut diagnostic = Diagnostic::new(self.file, code, &message, hint);
        if !key_path.is_empty() {
            diagnostic = diagnostic.at_path(key_path);
        }
        match at {
            Locator::Nowhere | Locator::FileLocation(None) => diagnostic,
            Locator::Key(key) => diagnostic.at_spot(locate_nth(self.text, key, 0)),
            Locator::Entry { list, index, key } => {
                diagnostic.at_spot(locate_entry_field(self.text, list, index, key))
            }
            Locator::FileLocation(Some(file_location)) => {
                let column = Some(file_location.column).filter(|column| *column > 0);
                diagnostic.at_line(file_location.line, column)
            }
        }
    }
}

/// One constraint a list entry must keep: whether the entry breaks it, the field it is about
/// (such as `dice.count`), the message and the hint.
pub(super) type EntryConstraint = (bool, &'static str, String, &'static str);

/// How an empty id is refused: the message and the hint, worded per pack file.
pub(super) type EmptyWords = (&'static str, &'static str);

/// How a duplicate id is refused: the words before and after the quoted id, and the hint.
pub(super) type DuplicateWords = (&'static str, &'static str, &'static str);

/// Where in its file a diagnostic is.
#[derive(Clone, Copy)]
pub(super) enum Locator<'a> {
    /// No line; the key path alone says where.
    Nowhere,
    /// The first `key` in the text.
    Key(&'a str),
    /// `key` inside the `index`th entry of the top-level `list` (such as `meters:`), counted
    /// in the text. Each entry starts at its `id:` key.
    Entry {
        list: &'a str,
        index: usize,
        key: &'a str,
    },
    /// A line and column serde recorded, already counted from the file's first line.
    FileLocation(Option<LineColumn>),
}

/// A message and its file location from a `compile_*` step that does not know its file yet.
#[derive(Debug)]
pub(super) struct CompileError {
    pub message: String,
    pub file_location: Option<LineColumn>,
}

impl From<String> for CompileError {
    fn from(message: String) -> Self {
        Self {
            message,
            file_location: None,
        }
    }
}

impl From<&str> for CompileError {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

/// Diagnostics in the order they were found.
#[derive(Default)]
pub(super) struct Diagnostics(Vec<Diagnostic>);

impl Diagnostics {
    /// The value, or `None` once the result's diagnostics are kept.
    pub(super) fn keep<T>(&mut self, result: Result<T, Vec<Diagnostic>>) -> Option<T> {
        result.map_err(|more| self.0.extend(more)).ok()
    }

    pub(super) fn push(&mut self, diagnostic: Diagnostic) {
        self.0.push(diagnostic);
    }

    /// `value` when nothing was found, else every diagnostic.
    pub(super) fn into_result<T>(self, value: T) -> Result<T, Vec<Diagnostic>> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(self.0)
        }
    }
}

impl Extend<Diagnostic> for Diagnostics {
    fn extend<I: IntoIterator<Item = Diagnostic>>(&mut self, more: I) {
        self.0.extend(more);
    }
}

/// The line and column serde recorded for a value, or `None` when it has no line.
pub(super) fn file_location_of<T>(span: &Spanned<T>) -> Option<LineColumn> {
    let line = u32::try_from(span.referenced.line()).unwrap_or(0);
    let column = u32::try_from(span.referenced.column()).unwrap_or(0);
    (line > 0).then_some(LineColumn { line, column })
}

/// `field` inside the `index`th entry of the top-level `list`. An entry starts at a line
/// whose key is `id:`, so an `id:` inside a value, or the file's own `id:`, starts none.
fn locate_entry_field(text: &str, list: &str, index: usize, field: &str) -> Option<(u32, u32)> {
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.iter().position(|line| line.starts_with(list))? + 1;
    let last = lines[first..]
        .iter()
        .position(|line| ends_list(line))
        .map_or(lines.len(), |end| first + end);
    let marks: Vec<usize> = (first..last)
        .filter(|&line_index| is_id_key(lines[line_index]))
        .collect();
    let start = *marks.get(index)?;
    let end = marks.get(index + 1).copied().unwrap_or(last);
    (start..end).find_map(|line_index| {
        let column = lines[line_index].find(field)?;
        Some((line_index as u32 + 1, column as u32 + 1))
    })
}

/// A line that ends a top-level list: the next top-level key, or the frontmatter fence.
fn ends_list(line: &str) -> bool {
    let line = line.trim_end();
    line == "---" || (!line.is_empty() && !line.starts_with([' ', '-', '#']))
}

/// A line whose key is `id:`, with or without the `- ` that opens a list item.
fn is_id_key(line: &str) -> bool {
    let line = line.trim_start();
    let line = line.strip_prefix('-').map_or(line, str::trim_start);
    line.starts_with("id:")
}
