use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::canon::{load_canon, Canon};
use crate::diagnostic::{locate_nth, reason_of, schema, Diagnostic};
use crate::error::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintHit {
    pub path: String,
    pub text: String,
}

/// Hidden fact texts found as case-sensitive substrings of a Markdown body.
/// An empty vec is a clean pack.
pub fn lint(root: &Path) -> Result<Vec<LintHit>, Reason> {
    let canon = load_canon(root).map_err(reason_of)?;
    let (leaks, problems) = scan(root, &canon);
    if !problems.is_empty() {
        return Err(reason_of(problems));
    }
    Ok(leaks
        .into_iter()
        .map(|leak| LintHit {
            path: leak.path,
            text: leak.text,
        })
        .collect())
}

/// Hidden-fact sentences found in Markdown bodies, plus files the scan could not read.
pub fn body_leaks(root: &Path, canon: &Canon) -> Vec<Diagnostic> {
    let (leaks, mut problems) = scan(root, canon);
    for leak in leaks {
        let mut diag = Diagnostic::new(
            &leak.path,
            "HIDDEN_FACT_IN_BODY",
            &format!("hidden fact '{}' is in the body", leak.text),
            "remove that sentence from the prose, or set hidden: false",
        );
        if let Some(line) = leak.line {
            diag = diag.at_line(line, None);
        }
        problems.push(diag);
    }
    problems
}

/// Hidden-fact sentences found in a message template's `text`.
/// `HIDDEN_FACT_IN_BODY` stays the Markdown scan. A move row is not a body.
pub fn message_text_leaks(root: &Path, canon: &Canon) -> Vec<Diagnostic> {
    let needles = hidden_needles(canon);
    if needles.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for rule in canon.rules.iter() {
        for effect in rule
            .do_
            .iter()
            .chain(rule.success.iter())
            .chain(rule.partial.iter())
            .chain(rule.failure.iter())
        {
            let Some(text) = effect.note_text() else {
                continue;
            };
            if needles.iter().any(|needle| text.contains(needle)) {
                hits.push((rule.file.clone(), text.to_string()));
            }
        }
    }
    hits.sort();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut problems = Vec::new();
    for (rel, text) in hits {
        let Ok(file) = fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        let nth = seen.entry(format!("{rel}:{text}")).or_insert(0);
        let spot = locate_nth(&file, &text, *nth);
        *nth += 1;
        for needle in &needles {
            if !text.contains(needle) {
                continue;
            }
            let mut diag = Diagnostic::new(
                &rel,
                "HIDDEN_FACT_IN_TEXT",
                &format!("hidden fact '{needle}' is in a note"),
                "rewrite that note, or set hidden: false",
            );
            if let Some((line, column)) = spot {
                diag = diag.at_line(line, Some(column));
            }
            problems.push(diag);
        }
    }
    problems
}

fn hidden_needles(canon: &Canon) -> Vec<String> {
    let mut needles: Vec<String> = canon
        .facts
        .values()
        .filter(|fact| fact.hidden && !fact.text.is_empty())
        .map(|fact| fact.text.clone())
        .collect();
    needles.sort();
    needles.dedup();
    needles
}

struct Leak {
    path: String,
    line: Option<u32>,
    text: String,
}

fn scan(root: &Path, canon: &Canon) -> (Vec<Leak>, Vec<Diagnostic>) {
    let mut needles: Vec<String> = canon
        .facts
        .values()
        .filter(|fact| fact.hidden)
        .map(|fact| fact.text.clone())
        .collect();
    needles.sort();
    needles.dedup();

    let mut files = Vec::new();
    let mut problems = Vec::new();
    if let Err(reason) = collect_markdown(root, root, &mut files) {
        problems.push(
            schema(
                "pack",
                "a Markdown file could not be read",
                "check that cast/ and world/ are readable UTF-8",
            )
            .with_path(reason.token()),
        );
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut leaks = Vec::new();
    for (rel, text) in files {
        let body = match markdown_body(&text) {
            Ok(body) => body,
            Err(_) => {
                problems.push(
                    schema(
                        &rel,
                        "this Markdown file has no frontmatter fence",
                        "wrap the header in --- lines",
                    )
                    .at_line(1, Some(1)),
                );
                continue;
            }
        };
        let body_at = text.find(body).unwrap_or(0);
        let base = text[..body_at].chars().filter(|ch| *ch == '\n').count() as u32;
        for needle in &needles {
            if !body.contains(needle) {
                continue;
            }
            let line = body
                .lines()
                .position(|line| line.contains(needle))
                .map(|index| base + index as u32 + 1);
            leaks.push(Leak {
                path: rel.clone(),
                line,
                text: needle.clone(),
            });
        }
    }
    (leaks, problems)
}

fn collect_markdown(
    dir: &Path,
    root: &Path,
    out: &mut Vec<(String, String)>,
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
            collect_markdown(&path, root, out)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| Reason::Schema)?
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let text = fs::read_to_string(&path).map_err(|_| Reason::Schema)?;
        out.push((rel, text));
    }
    Ok(())
}

fn markdown_body(text: &str) -> Result<&str, Reason> {
    let rest = text.strip_prefix("---\n").ok_or(Reason::Schema)?;
    let index = rest.find("\n---\n").ok_or(Reason::Schema)?;
    Ok(&rest[index + 5..])
}

/// Markdown with the body removed. Frontmatter stays. The trailing delimiter is kept.
pub fn strip_body(text: &str) -> Result<String, Reason> {
    let rest = text.strip_prefix("---\n").ok_or(Reason::Schema)?;
    let index = rest.find("\n---\n").ok_or(Reason::Schema)?;
    Ok(format!("---\n{}\n---\n", &rest[..index]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_body_rejects_a_missing_fence() {
        assert_eq!(strip_body("no fence\n"), Err(Reason::Schema));
        assert_eq!(strip_body("---\nno closer\n"), Err(Reason::Schema));
    }

    #[test]
    fn markdown_body_rejects_a_missing_fence() {
        assert_eq!(markdown_body("no fence\n"), Err(Reason::Schema));
        assert_eq!(markdown_body("---\nno closer\n"), Err(Reason::Schema));
    }
}
