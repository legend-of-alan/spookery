use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn hits(paths: &[PathBuf], needles: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for path in paths {
        let bytes = fs::read(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        for needle in needles {
            let needle_bytes = needle.as_bytes();
            if bytes.windows(needle_bytes.len()).any(|window| window == needle_bytes) {
                found.push(format!("{} contains {needle}", path.display()));
            }
        }
    }
    found
}

#[test]
fn import_boundaries() {
    let root = root();
    let kernel = root.join("src/kernel");
    let tick = root.join("src/tick");
    let src = root.join("src");
    assert!(kernel.is_dir(), "src/kernel is missing");
    assert!(tick.is_dir(), "src/tick is missing");

    let mut kernel_tick = Vec::new();
    files_under(&kernel, &mut kernel_tick);
    files_under(&tick, &mut kernel_tick);
    let kernel_tick_hits = hits(
        &kernel_tick,
        &["rmcp", "tokio", "sqlx", "postgres", "redis"],
    );
    assert!(
        kernel_tick_hits.is_empty(),
        "kernel or tick import boundary:\n{}",
        kernel_tick_hits.join("\n")
    );

    let mut src_files = Vec::new();
    files_under(&src, &mut src_files);
    let src_hits = hits(
        &src_files,
        &["langchain", "openai", "anthropic", "ollama", "reqwest"],
    );
    assert!(
        src_hits.is_empty(),
        "src import boundary:\n{}",
        src_hits.join("\n")
    );
}
