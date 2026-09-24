use crate::error::Code;

/// Markdown with the body removed. Frontmatter stays. The trailing delimiter is kept.
pub fn strip_body(text: &str) -> Result<String, Code> {
    let rest = text.strip_prefix("---\n").ok_or(Code::Schema)?;
    let index = rest.find("\n---\n").ok_or(Code::Schema)?;
    Ok(format!("---\n{}\n---\n", &rest[..index]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_body_rejects_a_missing_fence() {
        assert_eq!(strip_body("no fence\n"), Err(Code::Schema));
        assert_eq!(strip_body("---\nno closer\n"), Err(Code::Schema));
    }
}
