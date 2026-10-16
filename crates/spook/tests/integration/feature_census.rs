//! Each feature folder runs in one program, and the scenario count stays put.

use std::fs;
use std::path::Path;

use crate::feature_paths::{HOST, WORLD};

const SCENARIOS: usize = 333;

#[test]
fn every_feature_folder_runs_in_one_program() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/features");
    let mut found = Vec::new();
    for entry in fs::read_dir(&root).unwrap_or_else(|err| panic!("read {}: {err}", root.display()))
    {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            found.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    found.sort();
    let mut owned = owners();
    owned.sort();
    assert_eq!(found, owned, "a feature folder is missing or listed twice");
}

#[test]
fn feature_scenarios_stay_at_333() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/features");
    let mut total = 0;
    for entry in walk(&root) {
        let text = fs::read_to_string(&entry)
            .unwrap_or_else(|err| panic!("read {}: {err}", entry.display()));
        total += executed_scenarios(&text);
    }
    assert_eq!(total, SCENARIOS);
}

fn owners() -> Vec<String> {
    HOST.iter()
        .chain(WORLD)
        .map(|path| {
            Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().is_some_and(|ext| ext == "feature") {
            out.push(path);
        }
    }
    out
}

fn executed_scenarios(text: &str) -> usize {
    let mut scenarios = 0;
    let mut rows = 0;
    let mut in_examples = false;
    let mut header = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Scenario Outline:") || trimmed.starts_with("Scenario Template:") {
            in_examples = false;
            continue;
        }
        if trimmed.starts_with("Scenario:") || trimmed.starts_with("Example:") {
            scenarios += 1;
            in_examples = false;
            continue;
        }
        if trimmed == "Examples:" || trimmed.starts_with("Examples:") {
            in_examples = true;
            header = true;
            continue;
        }
        if !in_examples {
            continue;
        }
        if trimmed.starts_with('|') {
            if header {
                header = false;
            } else {
                rows += 1;
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            continue;
        }
        in_examples = false;
    }
    scenarios + rows
}
