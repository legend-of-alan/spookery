use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use cucumber::{given, then};

use crate::support;
use crate::World;

/// Kernel subsystems, bottom to top. A file may import a name on a lower row.
/// `state` is the planted row: it sits under `rules`, so `crate::rules` is refused.
/// `draft` is the staged now, under `rules`, which pays costs into it.
/// `lint` and `map` sit under `world`: world runs them while checking a pack, and they do not call world.
const LAYERS: &[&[&str]] = &[
    &["error", "format", "hash", "key"],
    &["diagnostic", "ids", "intent", "meter"],
    &["effect", "model", "rule"],
    &["check", "journal", "lock", "speech"],
    &["store"],
    &["canon", "state"],
    &["draft", "firewall"],
    &["rules"],
    &["lint", "map"],
    &["visibility"],
    &["world"],
    &["pause"],
    &["expand"],
    &["apply", "commit", "perceive"],
    // The player digest reads the view, so this row may import perceive.
    &["host_summary"],
];

pub(crate) fn clean_tree_holds() {
    let root = support::workspace_root();
    let down = downward_violations(&root.join("crates/kernel"));
    assert!(down.is_empty(), "{}", down.join("\n"));
}

fn downward_violations(kernel: &Path) -> Vec<String> {
    let mut files = Vec::new();
    support::rust_files(kernel, &mut files);
    let mut hits = Vec::new();
    for path in files {
        let owner = subsystem(kernel, &path);
        if owner == "lib" || owner == "api" {
            continue;
        }
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        for dep in crate_deps(&production_source(&text)) {
            if !may_import(&owner, &dep) {
                hits.push(format!("{} imports crate::{dep}", path.display()));
            }
        }
    }
    hits
}

fn may_import(from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    match (layer_of(from), layer_of(to)) {
        (Some(from_layer), Some(to_layer)) => to_layer < from_layer,
        _ => false,
    }
}

fn layer_of(name: &str) -> Option<usize> {
    LAYERS.iter().position(|row| row.contains(&name))
}

fn subsystem(kernel: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(kernel).unwrap_or(path);
    let stem = path.file_stem().unwrap().to_string_lossy();
    if stem != "mod" && layer_of(&stem).is_some() {
        return stem.to_string();
    }
    let first = rel
        .components()
        .next()
        .unwrap()
        .as_os_str()
        .to_string_lossy();
    first.trim_end_matches(".rs").to_string()
}

fn crate_deps(code: &str) -> BTreeSet<String> {
    let needle = "crate::";
    let bytes = code.as_bytes();
    let mut deps = BTreeSet::new();
    let mut start = 0;
    while let Some(at) = code[start..].find(needle) {
        let abs = start + at;
        if abs > 0 {
            let prev = bytes[abs - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                start = abs + needle.len();
                continue;
            }
        }
        let mut end = abs + needle.len();
        if end >= bytes.len() || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            start = abs + needle.len();
            continue;
        }
        let name_at = end;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        deps.insert(code[name_at..end].to_string());
        start = end;
    }
    deps
}

fn production_source(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if is_char_quote(bytes, i) {
            out.push(' ');
            i += 3;
            continue;
        }
        if is_line_comment(bytes, i) {
            i = skip_line(bytes, i);
            out.push('\n');
            continue;
        }
        if is_block_comment(bytes, i) {
            i = skip_block(bytes, i);
            out.push(' ');
            continue;
        }
        if bytes[i] == b'"' {
            let end = skip_string(bytes, i);
            out.push_str(&src[i..end]);
            i = end;
            continue;
        }
        if let Some(end) = cfg_test_mod_end(src, i) {
            out.push(' ');
            i = end;
            continue;
        }
        if let Some(end) = mod_tests_end(src, i) {
            out.push(' ');
            i = end;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn cfg_test_mod_end(src: &str, i: usize) -> Option<usize> {
    let rest = src.get(i..)?;
    let rest = rest.strip_prefix("#[cfg(test)]")?;
    let rest = rest.trim_start_matches([' ', '\t', '\r', '\n']);
    let rest = rest.strip_prefix("mod ")?;
    let mut name_len = 0;
    for byte in rest.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'_' {
            name_len += 1;
        } else {
            break;
        }
    }
    if name_len == 0 {
        return None;
    }
    let after = rest[name_len..].trim_start_matches([' ', '\t', '\r', '\n']);
    if !after.starts_with('{') {
        return None;
    }
    let open = src.len() - after.len();
    Some(skip_brace(src, open))
}

fn mod_tests_end(src: &str, i: usize) -> Option<usize> {
    if i > 0 {
        let prev = src.as_bytes()[i - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }
    let rest = src.get(i..)?.strip_prefix("mod tests")?;
    if rest
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    let after = rest.trim_start_matches([' ', '\t', '\r', '\n']);
    if !after.starts_with('{') {
        return None;
    }
    let open = src.len() - after.len();
    Some(skip_brace(src, open))
}

fn skip_brace(src: &str, open: usize) -> usize {
    let bytes = src.as_bytes();
    let mut i = open;
    let mut depth = 0;
    while i < bytes.len() {
        if is_char_quote(bytes, i) {
            i += 3;
            continue;
        }
        if is_line_comment(bytes, i) {
            i = skip_line(bytes, i);
            continue;
        }
        if is_block_comment(bytes, i) {
            i = skip_block(bytes, i);
            continue;
        }
        if bytes[i] == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if bytes[i] == b'{' {
            depth += 1;
        } else if bytes[i] == b'}' {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        }
        i += 1;
    }
    i
}

fn is_char_quote(bytes: &[u8], i: usize) -> bool {
    i + 2 < bytes.len() && bytes[i] == b'\'' && bytes[i + 1] == b'"' && bytes[i + 2] == b'\''
}

fn is_line_comment(bytes: &[u8], i: usize) -> bool {
    i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/'
}

fn is_block_comment(bytes: &[u8], i: usize) -> bool {
    i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*'
}

fn skip_line(bytes: &[u8], i: usize) -> usize {
    bytes
        .iter()
        .skip(i)
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |at| i + at)
}

fn skip_block(bytes: &[u8], i: usize) -> usize {
    let mut at = i + 2;
    while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
        at += 1;
    }
    (at + 2).min(bytes.len())
}

fn skip_string(bytes: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

#[given(expr = "kernel\\/state imports crate::rules")]
fn kernel_state_imports_rules(world: &mut World) {
    let dir = support::fresh_dir("down-plant");
    let kernel = dir.join("kernel");
    fs::create_dir_all(&kernel).unwrap_or_else(|err| panic!("mkdir {}: {err}", kernel.display()));
    let path = kernel.join("state.rs");
    fs::write(&path, "use crate::rules;\n")
        .unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    world.scratch = downward_violations(&kernel).join("\n");
    let _ = fs::remove_dir_all(&dir);
}

#[given(expr = "kernel\\/{word}.rs imports crate::{word}")]
fn kernel_file_imports(world: &mut World, name: String, dep: String) {
    let dir = support::fresh_dir("climb-plant");
    let kernel = dir.join("kernel");
    fs::create_dir_all(&kernel).unwrap_or_else(|err| panic!("mkdir {}: {err}", kernel.display()));
    let path = kernel.join(format!("{name}.rs"));
    fs::write(&path, format!("use crate::{dep};\n"))
        .unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    world.scratch = downward_violations(&kernel).join("\n");
    let _ = fs::remove_dir_all(&dir);
}

#[then(expr = "the boundary check names crate::{word}")]
fn the_boundary_check_names_the_import(world: &mut World, dep: String) {
    let needle = format!("crate::{dep}");
    assert!(
        world.scratch.contains(&needle),
        "boundary check did not name {needle}: {}",
        world.scratch
    );
}

#[then(expr = "the boundary check fails")]
fn the_boundary_check_fails(world: &mut World) {
    assert!(
        world.scratch.contains("crate::rules"),
        "boundary check accepted kernel/state importing crate::rules"
    );
}

#[then(expr = "the boundary check passes on main")]
fn the_boundary_check_passes_on_main(_world: &mut World) {
    clean_tree_holds();
}
