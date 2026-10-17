use std::fs;
use std::path::{Path, PathBuf};

use cucumber::then;
use serde_json::Value;

use crate::World;

#[then(expr = "CONTRIBUTING.md states issues only")]
fn contributing_states_issues_only(_world: &mut World) {
    let text = read_repo("CONTRIBUTING.md");
    let flat = text.to_ascii_lowercase();
    assert!(
        flat.contains("issues only"),
        "CONTRIBUTING.md does not state issues only:\n{text}"
    );
    assert!(
        flat.contains("closed with thanks"),
        "CONTRIBUTING.md does not close outside pull requests with thanks:\n{text}"
    );
    assert!(
        flat.contains("deferred")
            && flat.contains("contributor license")
            && flat.contains("certificate of origin"),
        "CONTRIBUTING.md does not defer the CLA and the DCO:\n{text}"
    );
    assert!(
        text.contains("Issues and PRs") && text.contains("README.md"),
        "CONTRIBUTING.md does not point at the README:\n{text}"
    );
}

#[then(expr = ".github\\/ISSUE_TEMPLATE holds bug, pack and feature forms")]
fn issue_template_holds_bug_pack_and_feature_forms(_world: &mut World) {
    let dir = find_repo_root().join(".github/ISSUE_TEMPLATE");
    for stem in ["bug", "pack", "feature"] {
        parse_form(&dir.join(format!("{stem}.yml")));
    }
    let bug = read_repo(".github/ISSUE_TEMPLATE/bug.yml").to_ascii_lowercase();
    assert!(
        bug.contains("save") && bug.contains("spook verify"),
        "the bug form does not ask for the save and spook verify output"
    );
    let config = parse_yaml(&dir.join("config.yml"));
    assert_eq!(
        config.get("blank_issues_enabled").and_then(Value::as_bool),
        Some(false),
        "blank issues are still enabled"
    );
}

fn parse_form(path: &Path) {
    let form = parse_yaml(path);
    let name = form.get("name").and_then(Value::as_str).unwrap_or("");
    assert!(
        name.chars().count() > 3,
        "{} needs a name longer than 3 characters",
        path.display()
    );
    assert!(
        form.get("description").and_then(Value::as_str).is_some(),
        "{} needs a description",
        path.display()
    );
    assert!(
        form.get("body")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()),
        "{} needs a form body",
        path.display()
    );
}

fn parse_yaml(path: &Path) -> Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_saphyr::from_str(&text)
        .unwrap_or_else(|err| panic!("{} is not yaml: {err}", path.display()))
}

pub(crate) fn read_repo(rel: &str) -> String {
    let path = find_repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

pub(crate) fn find_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
