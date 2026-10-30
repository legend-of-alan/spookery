//! Reading one pack file: its schema tag, its typed YAML, and where a problem sits in it.

use std::fs;
use std::path::Path;

use serde_saphyr::Spanned;

use crate::diagnostic::{
    coded, locate_nth, parse_file, schema as schema_error, tag_diagnostic, Diagnostic, Severity,
};
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
pub(super) const BUDGET_SCHEMA: &str = "budget";

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

    /// The refusal at `path` when `id` is empty.
    pub fn required(&self, id: &str, path: &str, at: At, words: Empty) -> Option<Diagnostic> {
        let (problem, hint) = words;
        id.is_empty()
            .then(|| self.refusal(path, at, problem.into(), hint))
    }

    /// The refusal at `path` for an `id` the caller found a second time.
    pub fn repeat(&self, id: &str, path: &str, at: At, words: Twice) -> Diagnostic {
        let (before, after, hint) = words;
        self.refusal(path, at, format!("{before} '{id}' {after}"), hint)
    }

    /// The refusal for a blank `display_name`, which places and cast both have.
    pub fn display_name(&self, name: &str) -> Option<Diagnostic> {
        let hint = "set display_name to the name players see";
        let empty = ("display_name is empty", hint);
        self.required(name.trim(), "display_name", At::Key("display_name:"), empty)
    }

    /// A converter's `fault` at the spot it recorded, refused as a schema error.
    pub fn fault(&self, path: &str, fault: Fault, hint: &str) -> Diagnostic {
        self.refusal(path, At::Spot(fault.spot), fault.message, hint)
    }

    fn refusal(&self, path: &str, at: At, problem: String, hint: &str) -> Diagnostic {
        Problem {
            source: self,
            path,
            at,
            problem,
            hint,
        }
        .schema()
    }
}

/// How an empty id is refused: the sentence and the hint, worded per kind.
pub(super) type Empty = (&'static str, &'static str);

/// How a repeated id is refused: the words before and after the quoted id, and the hint.
pub(super) type Twice = (&'static str, &'static str, &'static str);

/// Where a problem is.
#[derive(Clone, Copy)]
pub(super) enum At<'a> {
    /// No line; the path alone says where.
    Nowhere,
    /// The first `key` in the text, moved down by `line_base`.
    Key(&'a str),
    /// `key` inside the `row`th item of the top-level `list` (such as `facts:`), counted
    /// in the text as given, without `line_base`. Each item starts at its `id:` key.
    Row {
        list: &'a str,
        row: usize,
        key: &'a str,
    },
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
        let source = self.source;
        let code = Severity::Error(Reason::Schema);
        let mut diag = Diagnostic::new(source.file, code, &self.problem, self.hint);
        if !self.path.is_empty() {
            diag = diag.with_path(self.path);
        }
        match self.at {
            At::Nowhere | At::Spot(None) => diag,
            At::Key(key) => with_spot(diag, locate_nth(source.text, key, 0), source.line_base),
            At::Row { list, row, key } => {
                with_spot(diag, locate_row_field(source.text, list, row, key), 0)
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

/// Diagnostics in the order they were found.
#[derive(Default)]
pub(super) struct Diags(Vec<Diagnostic>);

impl Diags {
    /// The value, or `None` once the result's diagnostics are kept.
    pub fn keep<T>(&mut self, result: Result<T, Vec<Diagnostic>>) -> Option<T> {
        result.map_err(|more| self.0.extend(more)).ok()
    }

    pub fn push(&mut self, diag: Diagnostic) {
        self.0.push(diag);
    }

    /// `value` when nothing was found, else every diagnostic.
    pub fn result<T>(self, value: T) -> Result<T, Vec<Diagnostic>> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(self.0)
        }
    }
}

impl Extend<Diagnostic> for Diags {
    fn extend<I: IntoIterator<Item = Diagnostic>>(&mut self, more: I) {
        self.0.extend(more);
    }
}

/// The line and column serde recorded for a value, or `None` when it has no line.
pub(super) fn serde_spot<T>(span: &Spanned<T>) -> Option<Spot> {
    let line = u32::try_from(span.referenced.line()).unwrap_or(0);
    let column = u32::try_from(span.referenced.column()).unwrap_or(0);
    (line > 0).then_some(Spot { line, column })
}

/// `field` inside the `row`th item of the top-level `list`. An item starts at a line whose
/// key is `id:`, so an `id:` inside a value, or the file's own `id:`, is not a row.
fn locate_row_field(text: &str, list: &str, row: usize, field: &str) -> Option<(u32, u32)> {
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

/// A Markdown file split at its frontmatter fence. `text` is the whole file.
pub(super) struct Page {
    pub file: String,
    pub text: String,
    pub yaml: String,
    pub body: String,
}

/// Each `.md` file under `dir` in directory order, split at its frontmatter, or its UTF-8
/// or frontmatter refusal. An unreadable directory comes first; the files read before it
/// failed still follow.
pub(super) fn markdown_files(root: &Path, dir: &str) -> Vec<Result<Page, Vec<Diagnostic>>> {
    let mut files = Vec::new();
    let mut pages = Vec::new();
    if collect_files(&root.join(dir), root, &mut files).is_err() {
        pages.push(Err(vec![unreadable(dir)]));
    }
    for (file, bytes) in files {
        if !file.ends_with(".md") {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            pages.push(Err(vec![schema_error(
                &file,
                "this file is not UTF-8 text",
                "save it as UTF-8",
            )
            .with_path("schema")]));
            continue;
        };
        let split = split_frontmatter(&file, &text)
            .map(|(yaml, body)| (yaml.to_string(), body.to_string()));
        pages.push(split.map(|(yaml, body)| Page {
            file,
            text,
            yaml,
            body,
        }));
    }
    pages
}

pub(super) fn unreadable(dir: &str) -> Diagnostic {
    Problem {
        source: &Source::named(dir),
        path: dir,
        at: At::Nowhere,
        problem: format!("could not read {dir}/"),
        hint: "check that the directory is readable",
    }
    .schema()
}

pub(super) fn read_text(root: &Path, rel: &str) -> Result<String, Vec<Diagnostic>> {
    let path = root.join(rel);
    if !path.is_file() {
        return Err(vec![coded(
            rel,
            Reason::NotInSpec,
            &format!("{rel} is missing"),
            &format!("add {rel}"),
        )
        .with_path("schema")]);
    }
    fs::read_to_string(&path).map_err(|_| {
        vec![coded(
            rel,
            Reason::Schema,
            "this file could not be read as UTF-8 text",
            "save it as UTF-8",
        )
        .with_path("schema")]
    })
}

pub(super) fn collect_files(
    dir: &Path,
    root: &Path,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), Reason> {
    if !dir.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|_| Reason::Schema)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Reason::Schema)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, root, out)?;
            continue;
        }
        let rel = path.strip_prefix(root).map_err(|_| Reason::Schema)?;
        let rel = rel
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = fs::read(&path).map_err(|_| Reason::Schema)?;
        out.push((rel, bytes));
    }
    Ok(())
}

pub(super) fn split_frontmatter<'a>(
    file: &str,
    text: &'a str,
) -> Result<(&'a str, &'a str), Vec<Diagnostic>> {
    let rest = match text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    {
        Some(rest) => rest,
        None => {
            return Err(vec![schema_error(
                file,
                "this file has no YAML frontmatter",
                "start with a --- line and close the header with ---",
            )
            .at_line(1, Some(1))]);
        }
    };
    if let Some(index) = rest.find("\n---\n") {
        return Ok((&rest[..index], &rest[index + 5..]));
    }
    if let Some(index) = rest.find("\n---\r\n") {
        return Ok((&rest[..index], &rest[index + 6..]));
    }
    if let Some(yaml) = rest.strip_suffix("\n---") {
        return Ok((yaml, ""));
    }
    Err(vec![schema_error(
        file,
        "this file has no closing frontmatter fence",
        "close the header with a --- line",
    )
    .at_line(1, Some(1))])
}
