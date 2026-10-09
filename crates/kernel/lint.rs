use std::fs;
use std::path::Path;

use crate::canon::load_canon;
use crate::error::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintHit {
    pub path: String,
    pub text: String,
}

/// Hidden fact texts found as case-sensitive substrings of a Markdown body.
/// An empty vec is a clean pack.
pub fn lint(root: &Path) -> Result<Vec<LintHit>, Reason> {
    let canon = load_canon(root)?;
    let mut needles: Vec<String> = canon
        .facts
        .values()
        .filter(|fact| fact.hidden)
        .map(|fact| fact.text.clone())
        .collect();
    needles.sort();
    needles.dedup();

    let mut files = Vec::new();
    collect_markdown(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hits = Vec::new();
    for (rel, text) in files {
        let body = markdown_body(&text)?;
        for needle in &needles {
            if body.contains(needle) {
                hits.push(LintHit {
                    path: rel.clone(),
                    text: needle.clone(),
                });
            }
        }
    }
    Ok(hits)
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
