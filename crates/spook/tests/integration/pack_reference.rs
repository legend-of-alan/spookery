//! Every YAML fence in `docs/pack-reference.md` is one file of one pack.
//! `spook check` accepts that pack. A fence that drops a catalog key, or a
//! key the loader no longer accepts, fails this test.

use std::fs;
use std::path::{Path, PathBuf};

use spookery_engine::{check_pack, Code, Diagnostic, Reason};

const EFFECT_KEYS: &[&str] = &[
    "- meter:",
    "- flag:",
    "- location:",
    "- note:",
    "- narrate:",
    "- clock:",
    "- memory:",
    "- item:",
    "- reveal:",
    "- wield:",
    "- unwield:",
    "resist: same",
    "add:",
    "set:",
    "plus:",
];

const VERB_KEYS: &[&str] = &[
    "target: none",
    "target: actor",
    "target: place",
    "target: exit",
    "target: item",
    "via:",
    "partial_at:",
    "30s",
    "2h",
];

const EXIT_KEYS: &[&str] = &[
    "revealed_by:",
    "hidden: true",
    "locked_text:",
    "10m",
    "1d",
    "teleport: true",
];

const CONDITION_KEYS: &[&str] = &[
    "when: habit",
    "- all:",
    "- any:",
    "- not:",
    "has_item:",
    "wielding:",
    "note_exists:",
    "hour:",
    "players:",
    "- knows:",
    "- at:",
    "- clock:",
    "op: \"<\"",
    "op: \"<=\"",
    "op: \"==\"",
    "op: \">=\"",
    "op: \">\"",
];

struct Example {
    file: String,
    body: String,
}

struct PackDir {
    root: PathBuf,
}

impl PackDir {
    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for PackDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn examples_pass_spook_check() {
    let examples = read_examples();
    assert_catalog(&examples);
    let pack = write_pack(&examples, "clean");
    let found = check_pack(pack.path());
    assert!(found.is_empty(), "{}", report(&found));
}

#[test]
fn a_drifted_example_fails_spook_check() {
    let examples = read_examples();
    let clean = write_pack(&examples, "before-drift");
    let found = check_pack(clean.path());
    assert!(
        found.is_empty(),
        "drift compared against a pack that already fails\n{}",
        report(&found)
    );

    let mut drifted = examples;
    let verbs = drifted
        .iter_mut()
        .find(|example| example.file == "rules/verbs.yaml")
        .expect("verbs example");
    assert!(
        verbs.body.contains("\n    when:"),
        "verbs example has no when key to drift"
    );
    verbs.body = verbs.body.replacen("\n    when:", "\n    wen:", 1);
    let pack = write_pack(&drifted, "drift");
    let found = check_pack(pack.path());
    assert!(
        found
            .iter()
            .any(|diag| diag.code == Code::Error(Reason::Schema)),
        "{}",
        report(&found)
    );
}

fn assert_catalog(examples: &[Example]) {
    assert_keys(examples, "rules/verbs.yaml", EFFECT_KEYS);
    assert_keys(examples, "rules/verbs.yaml", VERB_KEYS);
    assert_keys(examples, "world/places/yard.md", EXIT_KEYS);
    assert_keys(examples, "cast/keeper.md", CONDITION_KEYS);
    assert!(
        examples
            .iter()
            .any(|example| example.file == "story/samples.yaml"),
        "missing example story/samples.yaml"
    );
}

fn assert_keys(examples: &[Example], file: &str, keys: &[&str]) {
    let body = body_of(examples, file);
    for key in keys {
        assert!(body.contains(key), "{file} is missing {key}");
    }
}

fn body_of(examples: &[Example], file: &str) -> String {
    examples
        .iter()
        .find(|example| example.file == file)
        .unwrap_or_else(|| panic!("missing example {file}"))
        .body
        .clone()
}

fn read_examples() -> Vec<Example> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/pack-reference.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read pack reference: {err}"));
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut examples = Vec::new();
    let mut rest = text.as_str();
    let opener = "```";
    while let Some(start) = rest.find(opener) {
        let after = &rest[start + opener.len()..];
        let (info, tail) = after
            .split_once('\n')
            .unwrap_or_else(|| panic!("unclosed fence"));
        let end = tail
            .find("\n```")
            .unwrap_or_else(|| panic!("fence is not closed: {info}"));
        if info.starts_with("yaml") {
            examples.push(parse_example(info, &tail[..end]));
        }
        rest = &tail[end + "\n```".len()..];
    }
    assert!(!examples.is_empty(), "pack reference has no YAML examples");
    let mut seen = std::collections::BTreeSet::new();
    for example in &examples {
        assert!(
            seen.insert(example.file.clone()),
            "duplicate example {}",
            example.file
        );
        assert!(
            example.body.contains("schema:"),
            "{} has no schema tag",
            example.file
        );
    }
    examples
}

fn parse_example(info: &str, body: &str) -> Example {
    let path = info
        .strip_prefix("yaml")
        .and_then(|rest| rest.trim().strip_prefix("file="))
        .unwrap_or_else(|| panic!("yaml fence needs file=: {info}"))
        .trim();
    assert!(
        !path.is_empty()
            && !path.starts_with('/')
            && !path.contains("..")
            && !path.contains('\\')
            && !path.contains(':')
            && !path.contains(char::is_whitespace),
        "example path escapes the pack: {path}"
    );
    let mut body = body.to_string();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    Example {
        file: path.to_string(),
        body,
    }
}

fn write_pack(examples: &[Example], label: &str) -> PackDir {
    let root = crate::support::fresh_dir(&format!("pack-ref-{label}"));
    for example in examples {
        let path = root.join(&example.file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|err| panic!("create parent: {err}"));
        }
        fs::write(&path, &example.body)
            .unwrap_or_else(|err| panic!("write {}: {err}", example.file));
    }
    PackDir { root }
}

fn report(found: &[Diagnostic]) -> String {
    found
        .iter()
        .map(Diagnostic::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}
