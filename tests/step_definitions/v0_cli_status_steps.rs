use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};

use crate::support::spook;
use crate::World;

const HOST: &str = "\
clock assassination_debt 2 6 paused
clock shop_day 1 1 running
";

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes =
                    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
}

fn file_key(path: &Path) -> String {
    format!("cli-file:{}", path.display())
}

fn assert_files_unchanged(w: &World, message: &str) {
    let pack = w.pack.as_ref().unwrap();
    let listed = files(pack);
    let count: usize = w
        .scratch
        .get("cli-file-count")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(listed.len(), count, "{message}");
    for (path, bytes) in listed {
        let saved = w
            .scratch
            .get(&file_key(&path))
            .unwrap_or_else(|| panic!("missing {}", path.display()));
        assert_eq!(saved, &hex(&bytes), "{message}: {}", path.display());
    }
}

#[given(expr = "the cli pack files are recorded")]
fn record_cli_files(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().clone();
    let listed = files(&pack);
    w.scratch
        .insert("cli-file-count".to_string(), listed.len().to_string());
    for (path, bytes) in listed {
        w.scratch.insert(file_key(&path), hex(&bytes));
    }
}

#[when(expr = "spook status runs on the pack")]
fn status_runs(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().clone();
    w.outputs
        .push(spook(&["status", pack.to_str().unwrap()]));
}

#[then(expr = "the status command exits 0")]
fn status_exits_0(w: &mut World) {
    let status = w.outputs.last().unwrap();
    assert!(
        status.status.success(),
        "status\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
}

#[then(expr = "the status command stdout is the host clocks")]
fn status_stdout(w: &mut World) {
    let status = w.outputs.last().unwrap();
    assert_eq!(String::from_utf8_lossy(&status.stdout), HOST);
}

#[then(expr = "status leaves the cli pack files unchanged")]
fn status_unchanged(w: &mut World) {
    assert_files_unchanged(w, "status wrote the pack");
}

#[then(expr = "schema failure leaves the cli pack files unchanged")]
fn schema_unchanged(w: &mut World) {
    assert_files_unchanged(w, "schema failure wrote the pack");
}
