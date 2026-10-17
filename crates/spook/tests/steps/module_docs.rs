use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then};

use crate::support;
use crate::World;

#[given(expr = "a source file with no module doc")]
fn a_source_file_with_no_module_doc(world: &mut World) {
    let dir = support::fresh_dir("module-doc");
    let path = dir.join("bare.rs");
    fs::write(&path, "fn bare() {}\n")
        .unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    let problems = module_doc_problems(&[path]);
    let _ = fs::remove_dir_all(&dir);
    world.scratch = problems.join("\n");
}

#[then(expr = "the module-doc check fails naming that file")]
fn the_module_doc_check_fails_naming_that_file(world: &mut World) {
    assert!(
        world.scratch.contains("bare.rs"),
        "module-doc check did not name the file: {}",
        world.scratch
    );
}

#[then(expr = "the module-doc check passes on main")]
fn the_module_doc_check_passes_on_main(_world: &mut World) {
    let mut files = Vec::new();
    rust_sources(&workspace_root().join("crates"), &mut files);
    files.sort();
    let problems = module_doc_problems(&files);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
            .path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) == Some("tests") {
                continue;
            }
            rust_sources(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn module_doc_problems(paths: &[PathBuf]) -> Vec<String> {
    let mut problems = Vec::new();
    for path in paths {
        let text =
            fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let shown = show(path);
        let Some(doc) = leading_module_doc(&text) else {
            problems.push(format!("{shown} has no module doc"));
            continue;
        };
        if path.file_name().and_then(|name| name.to_str()) == Some("lib.rs")
            && (!doc.contains("Public API") || !doc.contains("Internal"))
        {
            problems.push(format!("{shown} does not list Public API and Internal"));
        }
    }
    problems
}

fn leading_module_doc(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let first = lines.next()?;
    if !first.trim_start().starts_with("//!") {
        return None;
    }
    let mut doc = String::from(first);
    for line in lines {
        if line.trim_start().starts_with("//!") {
            doc.push('\n');
            doc.push_str(line);
        } else {
            break;
        }
    }
    Some(doc)
}

fn show(path: &Path) -> String {
    path.strip_prefix(workspace_root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
