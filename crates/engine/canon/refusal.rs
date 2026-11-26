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

    /// The refusal at `path` when `id` is empty.
    pub(super) fn refuse_empty(
        &self,
        id: &str,
        path: &str,
        at: Locator,
        words: EmptyWords,
    ) -> Option<Diagnostic> {
        let (problem, hint) = words;
        id.is_empty()
            .then(|| self.refuse_at(path, at, problem.into(), hint))
    }

    /// The refusal at `path` for an `id` the caller found a second time.
    pub(super) fn refuse_duplicate(
        &self,
        id: &str,
        path: &str,
        at: Locator,
        words: DuplicateWords,
    ) -> Diagnostic {
        let (before, after, hint) = words;
        self.refuse_at(path, at, format!("{before} '{id}' {after}"), hint)
    }

    /// The refusals for the `row`th item of the top-level `{noun}s:` list, whose id is
    /// `id`: an empty id, then each rule the row breaks, then, when nothing else is wrong
    /// and `taken` says an earlier row has this id, the repeat.
    pub(super) fn refuse_entry(
        &self,
        noun: &str,
        row: usize,
        id: &str,
        taken: bool,
        rules: impl IntoIterator<Item = EntryConstraint>,
    ) -> Vec<Diagnostic> {
        let list = format!("{noun}s:");
        let refuse = |field: &str, message: String, hint: &str| {
            let key = format!(
                "{}:",
                field.rsplit_once('.').map_or(field, |(_, last)| last)
            );
            let at = Locator::Entry {
                list: &list,
                row,
                key: &key,
            };
            self.refuse_at(&format!("{noun}s[{row}].{field}"), at, message, hint)
        };
        let empty = format!("a {noun} id is empty");
        let broken = rules.into_iter().filter(|(broken, ..)| *broken);
        let mut found: Vec<Diagnostic> = id
            .is_empty()
            .then(|| refuse("id", empty, &format!("set id to a short {noun} id")))
            .into_iter()
            .chain(broken.map(|(_, field, message, hint)| refuse(field, message, hint)))
            .collect();
        if found.is_empty() && taken {
            let twice = format!("{noun} '{id}' is listed twice");
            found.push(refuse("id", twice, &format!("keep one {noun}")));
        }
        found
    }

    /// A `CompileError` at the spot it recorded, refused as a schema error.
    pub(super) fn refuse_error(&self, path: &str, fault: CompileError) -> Diagnostic {
        self.refuse_at(path, Locator::FileLocation(fault.spot), fault.message, "")
    }

    fn refuse_at(&self, path: &str, at: Locator, problem: String, hint: &str) -> Diagnostic {
        Problem {
            source: self,
            path,
            at,
            message: problem,
            hint,
        }
        .into_diagnostic()
    }
}

/// One constraint a list entry must keep: whether the entry breaks it, the field it is about
/// (such as `dice.count`), the message and the hint.
pub(super) type EntryConstraint = (bool, &'static str, String, &'static str);

/// How an empty id is refused: the sentence and the hint, worded per kind.
pub(super) type EmptyWords = (&'static str, &'static str);

/// How a repeated id is refused: the words before and after the quoted id, and the hint.
pub(super) type DuplicateWords = (&'static str, &'static str, &'static str);

/// Where a problem is.
#[derive(Clone, Copy)]
pub(super) enum Locator<'a> {
    /// No line; the path alone says where.
    Nowhere,
    /// The first `key` in the text.
    Key(&'a str),
    /// `key` inside the `row`th item of the top-level `list` (such as `meters:`), counted
    /// in the text. Each item starts at its `id:` key.
    Entry {
        list: &'a str,
        row: usize,
        key: &'a str,
    },
    /// A line and column serde recorded, already counted from the file's first line.
    FileLocation(Option<LineColumn>),
}

/// One problem in one pack file. `path` is `""` when there is none.
pub(super) struct Problem<'a> {
    pub source: &'a FileText<'a>,
    pub path: &'a str,
    pub at: Locator<'a>,
    pub message: String,
    pub hint: &'a str,
}

impl Problem<'_> {
    pub(super) fn into_diagnostic(self) -> Diagnostic {
        let source = self.source;
        let code = Code::Error(Reason::Schema);
        let mut diag = Diagnostic::new(source.file, code, &self.message, self.hint);
        if !self.path.is_empty() {
            diag = diag.at_path(self.path);
        }
        match self.at {
            Locator::Nowhere | Locator::FileLocation(None) => diag,
            Locator::Key(key) => diag.at_spot(locate_nth(source.text, key, 0)),
            Locator::Entry { list, row, key } => {
                diag.at_spot(locate_entry_field(source.text, list, row, key))
            }
            Locator::FileLocation(Some(spot)) => {
                diag.at_line(spot.line, Some(spot.column).filter(|column| *column > 0))
            }
        }
    }
}

/// A message and its line from a `compile_*` step that does not know its file yet.
#[derive(Debug)]
pub(super) struct CompileError {
    pub message: String,
    pub spot: Option<LineColumn>,
}

impl From<String> for CompileError {
    fn from(message: String) -> Self {
        Self {
            message,
            spot: None,
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

    pub(super) fn push(&mut self, diag: Diagnostic) {
        self.0.push(diag);
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

/// `field` inside the `row`th item of the top-level `list`. An item starts at a line whose
/// key is `id:`, so an `id:` inside a value, or the file's own `id:`, is not a row.
fn locate_entry_field(text: &str, list: &str, row: usize, field: &str) -> Option<(u32, u32)> {
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.iter().position(|line| line.starts_with(list))? + 1;
    let last = lines[first..]
        .iter()
        .position(|line| ends_list(line))
        .map_or(lines.len(), |end| first + end);
    let marks: Vec<usize> = (first..last)
        .filter(|&index| is_id_key(lines[index]))
        .collect();
    let start = *marks.get(row)?;
    let end = marks.get(row + 1).copied().unwrap_or(last);
    (start..end).find_map(|index| {
        let column = lines[index].find(field)?;
        Some((index as u32 + 1, column as u32 + 1))
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
