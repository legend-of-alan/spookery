use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};

use crate::support::spook;
use crate::World;

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

#[given(expr = "a docks table")]
fn a_docks_table(world: &mut World) {
    let pack = crate::support::temp_pack("status");
    world.before = files(&pack);
    world.pack = Some(pack);
}

#[when(expr = "the operator runs status")]
fn the_operator_runs_status(world: &mut World) {
    let pack = world
        .pack
        .as_ref()
        .unwrap_or_else(|| panic!("no docks table"));
    let output = spook(&["status", pack.to_str().unwrap()]);
    world.code = output.status.code();
    world.scratch = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[then(expr = "status exits 0")]
fn status_exits_0(world: &mut World) {
    assert_eq!(world.code, Some(0), "{}", world.scratch);
}

#[then(expr = "the pack bytes are unchanged")]
fn the_pack_bytes_are_unchanged(world: &mut World) {
    let pack = world
        .pack
        .as_ref()
        .unwrap_or_else(|| panic!("no docks table"));
    assert_eq!(files(pack), world.before, "status wrote the pack");
}
