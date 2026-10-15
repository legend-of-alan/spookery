//! Every feature file cited by `docs/engine-guarantees.md` exists, and every
//! promise section cites at least one.

use std::fs;
use std::path::Path;

const STUB: &str = "Trigger delivery: TBD (issue #134)";

#[test]
fn cited_feature_files_exist() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let doc_path = root.join("docs/engine-guarantees.md");
    let doc = fs::read_to_string(&doc_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", doc_path.display()));
    assert!(
        doc.contains(STUB),
        "the trigger-delivery stub is missing from docs/engine-guarantees.md"
    );

    let mut cited = Vec::new();
    let mut sections = 0usize;
    for section in doc.split("\n## ").skip(1) {
        sections += 1;
        let heading = section.lines().next().unwrap_or(section);
        let found = citations(section);
        assert!(
            !found.is_empty(),
            "section `{heading}` cites no feature file"
        );
        cited.extend(found);
    }
    assert!(
        sections > 0,
        "docs/engine-guarantees.md has no promise sections"
    );

    let mut missing = Vec::new();
    for cite in &cited {
        let path = root.join("crates/spook/tests").join(cite);
        if !path.is_file() {
            missing.push(cite.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "cited feature file does not exist:\n{}",
        missing.join("\n")
    );
}

fn citations(section: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = section;
    while let Some(start) = rest.find("`features/") {
        let after = &rest[start + 1..];
        let end = after
            .find('`')
            .unwrap_or_else(|| panic!("unclosed feature citation in:\n{section}"));
        let cite = &after[..end];
        assert!(
            cite.ends_with(".feature"),
            "citation is not a feature file: {cite}"
        );
        assert!(
            !cite.contains('\\') && !cite.contains(".."),
            "citation is not a feature path: {cite}"
        );
        out.push(cite.to_string());
        rest = &after[end + 1..];
    }
    out
}
