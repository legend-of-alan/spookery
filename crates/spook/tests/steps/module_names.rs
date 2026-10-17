use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cucumber::then;

use crate::World;

const LANGUAGE_ROOTS: &[&str] = &["lib.rs", "main.rs", "mod.rs"];

#[then(expr = "no two spook modules share a file name besides mod.rs, lib.rs, and main.rs")]
fn no_two_spook_modules_share_a_file_name(_world: &mut World) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut by_name = BTreeMap::new();
    collect(root, root, &mut by_name);
    let shared = shared_names(&by_name);
    assert!(shared.is_empty(), "shared module file names:\n{shared}");
}

fn collect(root: &Path, dir: &Path, by_name: &mut BTreeMap<String, Vec<String>>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
            .path();
        if path.is_dir() {
            collect(root, &path, by_name);
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".rs") || LANGUAGE_ROOTS.contains(&name) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        by_name.entry(name.to_string()).or_default().push(rel);
    }
}

fn shared_names(by_name: &BTreeMap<String, Vec<String>>) -> String {
    let mut lines = Vec::new();
    for (name, paths) in by_name {
        if paths.len() < 2 {
            continue;
        }
        let mut paths = paths.clone();
        paths.sort();
        lines.push(format!("{name}: {}", paths.join(", ")));
    }
    lines.join("\n")
}
