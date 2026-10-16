//! Each feature folder runs in exactly one program, and no feature file sits outside a folder.

use std::fs;
use std::path::{Path, PathBuf};

use crate::feature_paths::{HOST, WORLD};

#[test]
fn every_feature_folder_runs_in_one_program() {
    let mut found = Vec::new();
    for entry in entries() {
        if entry.is_dir() {
            found.push(entry.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    found.sort();
    let mut owned = owners();
    owned.sort();
    assert_eq!(found, owned, "a feature folder is missing or listed twice");
}

#[test]
fn no_feature_file_sits_in_the_features_root() {
    let loose: Vec<PathBuf> = entries()
        .into_iter()
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "feature"))
        .collect();
    assert!(
        loose.is_empty(),
        "move these into a feature folder: {loose:?}"
    );
}

fn entries() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/features");
    fs::read_dir(&root)
        .unwrap_or_else(|err| panic!("read {}: {err}", root.display()))
        .map(|entry| entry.unwrap().path())
        .collect()
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
