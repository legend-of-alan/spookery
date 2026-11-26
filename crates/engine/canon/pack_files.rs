//! The pack directory as file text, and each Markdown page split at its frontmatter.

use std::fs;
use std::path::Path;

use crate::canon::refusal::{FileText, Locator};
use crate::diagnostic::{error, schema_error, Diagnostic};
use crate::Reason;

/// A Markdown file split at its frontmatter fence. `text` is the whole file, and
/// `frontmatter` starts at the opening fence, so its line numbers are the file's.
pub(super) struct Page {
    pub file: String,
    pub text: String,
    pub frontmatter: String,
    pub body: String,
}

/// Each `.md` file under `dir` in directory order, split at its frontmatter, or its UTF-8
/// or frontmatter refusal. An unreadable directory comes first; the files read before it
/// failed still follow.
pub(super) fn collect_markdown_files(root: &Path, dir: &str) -> Vec<Result<Page, Vec<Diagnostic>>> {
    let mut files = Vec::new();
    let mut pages = Vec::new();
    if collect_files(&root.join(dir), root, &mut files).is_err() {
        pages.push(Err(vec![refuse_unreadable_dir(dir)]));
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
            .at_key_path("schema")]));
            continue;
        };
        let split = split_frontmatter(&file, &text)
            .map(|(frontmatter, body)| (frontmatter.to_string(), body.to_string()));
        pages.push(split.map(|(frontmatter, body)| Page {
            file,
            text,
            frontmatter,
            body,
        }));
    }
    pages
}

pub(super) fn refuse_unreadable_dir(dir: &str) -> Diagnostic {
    FileText::without_text(dir).refuse_at(
        dir,
        Locator::Nowhere,
        format!("could not read {dir}/"),
        "check that the directory is readable",
    )
}

pub(super) fn read_text(root: &Path, file: &str) -> Result<String, Vec<Diagnostic>> {
    let file_path = root.join(file);
    if !file_path.is_file() {
        return Err(vec![error(
            file,
            Reason::NotInSpec,
            &format!("{file} is missing"),
            &format!("add {file}"),
        )
        .at_key_path("schema")]);
    }
    fs::read_to_string(&file_path).map_err(|_| {
        vec![error(
            file,
            Reason::Schema,
            "this file could not be read as UTF-8 text",
            "save it as UTF-8",
        )
        .at_key_path("schema")]
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
        let file_path = entry.path();
        if file_path.is_dir() {
            collect_files(&file_path, root, out)?;
            continue;
        }
        let file = file_path.strip_prefix(root).map_err(|_| Reason::Schema)?;
        let file = file
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = fs::read(&file_path).map_err(|_| Reason::Schema)?;
        out.push((file, bytes));
    }
    Ok(())
}

/// The frontmatter, opening fence included and closing fence left out, and the body after it.
fn split_frontmatter<'a>(file: &str, text: &'a str) -> Result<(&'a str, &'a str), Vec<Diagnostic>> {
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
    let fence_end = text.len() - rest.len();
    if let Some(index) = rest.find("\n---\n") {
        return Ok((&text[..fence_end + index], &rest[index + 5..]));
    }
    if let Some(index) = rest.find("\n---\r\n") {
        return Ok((&text[..fence_end + index], &rest[index + 6..]));
    }
    if rest.ends_with("\n---") {
        return Ok((&text[..text.len() - 4], ""));
    }
    Err(vec![schema_error(
        file,
        "this file has no closing frontmatter fence",
        "close the header with a --- line",
    )
    .at_line(1, Some(1))])
}
