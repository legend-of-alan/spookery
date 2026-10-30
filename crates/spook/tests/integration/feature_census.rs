//! Each feature folder runs in exactly one program, and no feature file sits outside a folder.

use std::fs;
use std::path::{Path, PathBuf};

use spookery_engine::testing as engine;

use crate::feature_dirs::{HOST, WORLD};

#[test]
fn every_feature_folder_runs_in_one_program() {
    assert_folders_match("tests/features", &[HOST, WORLD]);
    assert_folders_match("../engine/tests/features", &[engine::HOST, engine::WORLD]);
}

#[test]
fn no_feature_file_sits_in_the_features_root() {
    assert_no_loose_features("tests/features");
    assert_no_loose_features("../engine/tests/features");
}

fn assert_folders_match(rel: &str, lists: &[&[&str]]) {
    let mut found = Vec::new();
    for entry in entries(rel) {
        if entry.is_dir() {
            found.push(entry.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    found.sort();
    let mut owned = listed_folders(lists);
    owned.sort();
    assert_eq!(
        found, owned,
        "a feature folder in {rel} is missing or listed twice"
    );
}

fn assert_no_loose_features(rel: &str) {
    let loose: Vec<PathBuf> = entries(rel)
        .into_iter()
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "feature"))
        .collect();
    assert!(
        loose.is_empty(),
        "move these into a feature folder: {loose:?}"
    );
}

fn entries(rel: &str) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_dir(&root)
        .unwrap_or_else(|err| panic!("read {}: {err}", root.display()))
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn listed_folders(lists: &[&[&str]]) -> Vec<String> {
    lists
        .iter()
        .flat_map(|list| list.iter())
        .map(|path| {
            Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}
