use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Names that must not be `pub` on the kernel surface.
const NARROWED: &[&str] = &[
    "SAVE_EMIT_OPTIONS",
    "actor_of",
    "admit",
    "allows",
    "at_line",
    "at_start",
    "blake2s_hex",
    "body_leaks",
    "body_of_fact",
    "can_pay",
    "class_name",
    "compile",
    "defaults",
    "deserialize",
    "duration_minutes",
    "emit_effect_payload",
    "emit_memory_line",
    "emit_migrate",
    "emit_presence",
    "from_token",
    "habits_of",
    "habits_on",
    "hour_in",
    "ignore_resist",
    "is_eligible",
    "kernel_intent_id",
    "load_checks",
    "load_meters",
    "load_world_settings",
    "message_text_leaks",
    "migrate_file",
    "note_text",
    "parse_effect",
    "parse_migrate",
    "parse_presence",
    "parse_tick_effects",
    "player_verbs",
    "preimage",
    "presence_file",
    "release_wielded",
    "replaced_moves_file",
    "require_recorded_rules",
    "resolve_item_id",
    "rule_roll",
    "serialize",
    "set_pause_view",
    "shift_count",
    "stamp_now_hash",
    "teleport_in_verbs",
    "track",
    "track_before",
    "track_many",
    "used_exit",
    "uses_teleport",
    "visit",
    "with_path",
    "world_hour",
];

pub(crate) fn clean_tree_holds() {
    let root = support::workspace_root();
    let mut outside = Vec::new();
    support::rust_files(&root.join("crates/spook/src"), &mut outside);
    support::rust_files(&root.join("crates/spook/tests"), &mut outside);
    support::rust_files(&root.join("crates/round"), &mut outside);
    support::rust_files(&root.join("crates/kernel/tests"), &mut outside);
    let api = api_violations(&outside);
    assert!(api.is_empty(), "{}", api.join("\n"));
    let down = downward_violations(&root.join("crates/kernel"));
    assert!(down.is_empty(), "{}", down.join("\n"));
}

fn api_violations(files: &[PathBuf]) -> Vec<String> {
    let mut hits = Vec::new();
    for path in files {
        let text =
            fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let code = without_comments_or_strings(&text);
        for import in kernel_paths(&code) {
            if import == "spookery_kernel::api" || import.starts_with("spookery_kernel::api::") {
                continue;
            }
            hits.push(format!("{} imports {import}", path.display()));
        }
    }
    hits
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

fn kernel_paths(code: &str) -> Vec<String> {
    let needle = "spookery_kernel::";
    let bytes = code.as_bytes();
    let mut hits = Vec::new();
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
        loop {
            if end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                {
                    end += 1;
                }
                continue;
            }
            if code[end..].starts_with("::") {
                end += 2;
                continue;
            }
            break;
        }
        while code[..end].ends_with("::") {
            end -= 2;
        }
        if end > abs {
            hits.push(code[abs..end].to_string());
        }
        start = end.max(abs + needle.len());
    }
    hits
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

fn without_comments_or_strings(src: &str) -> String {
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
            i = skip_string(bytes, i);
            out.push(' ');
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
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

#[given(expr = "a spook source file that imports spookery_kernel::shard")]
fn a_spook_file_imports_shard(world: &mut World) {
    let dir = support::fresh_dir("api-plant");
    let path = dir.join("plant.rs");
    fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("mkdir {}: {err}", dir.display()));
    fs::write(&path, "use spookery_kernel::shard;\n")
        .unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    world.scratch = api_violations(&[path]).join("\n");
    let _ = fs::remove_dir_all(&dir);
}

#[then(expr = "the boundary check fails naming that file and path")]
fn the_boundary_check_names_the_file_and_path(world: &mut World) {
    assert!(
        world.scratch.contains("plant.rs"),
        "boundary check did not name the file: {}",
        world.scratch
    );
    assert!(
        world.scratch.contains("spookery_kernel::shard"),
        "boundary check did not name the import: {}",
        world.scratch
    );
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

#[then(expr = "the listed kernel functions are crate-private or gone")]
fn the_listed_kernel_functions_are_crate_private_or_gone(_world: &mut World) {
    let leaks = public_kernel_fns(NARROWED);
    assert!(leaks.is_empty(), "{}", leaks.join("\n"));
}

#[then(expr = "sort_unique stays public for the firewall tests")]
fn sort_unique_stays_public_for_the_firewall_tests(_world: &mut World) {
    let hits = public_kernel_fns(&["sort_unique"]);
    assert!(
        !hits.is_empty(),
        "sort_unique is not public, and the firewall tests call it"
    );
}

fn public_kernel_fns(names: &[&str]) -> Vec<String> {
    let mut files = Vec::new();
    support::rust_files(&support::workspace_root().join("crates/kernel"), &mut files);
    let mut hits = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let code_only = without_comments_or_strings(&production_source(&text));
        let mut in_use = false;
        for (index, line) in code_only.lines().enumerate() {
            let code = line.trim();
            if code.starts_with("pub use ") {
                in_use = true;
            }
            for name in names {
                if is_public_item(code) && declares_fn(code, name) {
                    hits.push(format!("{}:{} publishes {name}", path.display(), index + 1));
                } else if in_use && has_ident(code, name) {
                    hits.push(format!(
                        "{}:{} re-exports {name}",
                        path.display(),
                        index + 1
                    ));
                }
            }
            if code.contains(';') {
                in_use = false;
            }
        }
    }
    hits
}

fn is_public_item(code: &str) -> bool {
    code.starts_with("pub ")
}

fn declares_fn(code: &str, name: &str) -> bool {
    has_ident(code, &format!("fn {name}"))
}

fn has_ident(code: &str, name: &str) -> bool {
    let bytes = code.as_bytes();
    let needle = name.as_bytes();
    let mut start = 0;
    while let Some(at) = code[start..].find(name) {
        let abs = start + at;
        let before_ok = abs == 0 || !is_ident_byte(bytes[abs - 1]);
        let after = abs + needle.len();
        let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        start = abs + 1;
    }
    false
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
