use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use cucumber::{then, when};
use serde_json::Value;

use crate::World;

use super::table;

const DIAGNOSTIC_KEYS: [&str; 7] = ["file", "line", "column", "path", "code", "message", "hint"];

#[when(expr = "the operator runs spook check --json again")]
fn the_operator_runs_spook_check_json_again(world: &mut World) {
    world.scratch = world.stdout.clone();
    super::common::run_check_json(world);
}

#[when(expr = "the street exit is removed")]
fn the_street_exit_is_removed(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    super::common::replace_file(
        &pack.join("world/places/docks_shop.md"),
        "  - to: docks_street\n    name: docks_street\n",
        "",
    );
}

#[when(expr = "travel in verbs.yaml teleports")]
fn travel_in_verbs_yaml_teleports(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    super::common::replace_file(
        &pack.join("rules/verbs.yaml"),
        "          via: target\n",
        "          via: target\n          teleport: true\n",
    );
}

#[when(expr = "the save hash is restamped")]
fn the_save_hash_is_restamped(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let path = pack.join("state/save.yaml");
    let mut now = spookery_engine::load_save(&fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|err| panic!("load save: {err:?}"));
    now.pack_rev = spookery_engine::pack_rev(&pack).unwrap_or_else(|err| panic!("{err:?}"));
    fs::write(&path, spookery_engine::emit_save(&now).unwrap()).unwrap();
}

#[then(expr = "the report names {string}")]
fn the_report_names(world: &mut World, text: String) {
    assert!(
        world.stderr.contains(&text),
        "missing {text}\n{}",
        world.stderr
    );
}

#[then(expr = "the report has a location for {string}")]
fn the_report_has_a_location_for(world: &mut World, file: String) {
    let located = world.stderr.lines().any(|line| {
        let Some(rest) = line.strip_prefix(&file) else {
            return false;
        };
        rest.starts_with(':')
            && line.contains('(')
            && (rest.contains(": ")
                || rest[1..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_digit()))
    });
    assert!(located, "no location for {file}\n{}", world.stderr);
}

#[then(expr = "stdout is a JSON array of diagnostics")]
fn stdout_is_a_json_array_of_diagnostics(world: &mut World) {
    for item in json_stdout(world) {
        let object = item
            .as_object()
            .unwrap_or_else(|| panic!("not an object: {item}"));
        for key in DIAGNOSTIC_KEYS {
            assert!(object.contains_key(key), "missing {key} in {item}");
        }
        assert_eq!(object.len(), DIAGNOSTIC_KEYS.len(), "{item}");
        assert!(item["file"].is_string(), "{item}");
        assert!(item["code"].is_string(), "{item}");
        assert!(item["message"].is_string(), "{item}");
    }
}

#[then(
    expr = "a JSON diagnostic has code {string} file {string} path {string} and null line and column"
)]
fn a_json_diagnostic_has_code_file_path_and_null_line(
    world: &mut World,
    code: String,
    file: String,
    path: String,
) {
    let found = json_stdout(world).into_iter().any(|item| {
        item["code"].as_str() == Some(code.as_str())
            && item["file"].as_str() == Some(file.as_str())
            && item["path"].as_str() == Some(path.as_str())
            && item["line"].is_null()
            && item["column"].is_null()
    });
    assert!(
        found,
        "missing {code} in {file} at {path}\n{}",
        world.stdout
    );
}

#[then(
    expr = "a JSON diagnostic has code {string} file {string} path {string} at line {int} column {int}"
)]
fn a_json_diagnostic_has_code_file_path_at_line(
    world: &mut World,
    code: String,
    file: String,
    path: String,
    line: u64,
    column: u64,
) {
    let found = json_stdout(world).into_iter().any(|item| {
        item["code"].as_str() == Some(code.as_str())
            && item["file"].as_str() == Some(file.as_str())
            && item["path"].as_str() == Some(path.as_str())
            && item["line"].as_u64() == Some(line)
            && item["column"].as_u64() == Some(column)
    });
    assert!(
        found,
        "missing {code} in {file} at {path} line {line} column {column}\n{}",
        world.stdout
    );
}

#[then(expr = "the JSON diagnostics are ordered by file, line, and path")]
fn the_json_diagnostics_are_ordered(world: &mut World) {
    let rows = json_stdout(world);
    let keys: Vec<_> = rows.iter().map(json_sort_key).collect();
    let mut ordered = keys.clone();
    ordered.sort();
    assert_eq!(keys, ordered, "{}", world.stdout);
    let files: BTreeSet<_> = rows
        .iter()
        .filter_map(|item| item["file"].as_str())
        .collect();
    assert!(files.len() >= 2, "expected two files, got {files:?}");
}

#[then(expr = "stdout matches the earlier JSON")]
fn stdout_matches_the_earlier_json(world: &mut World) {
    assert_eq!(world.stdout, world.scratch);
}

#[then(expr = "the JSON array is one diagnostic code {string} file {string} path {string}")]
fn the_json_array_is_one_diagnostic(world: &mut World, code: String, file: String, path: String) {
    let rows = json_stdout(world);
    assert_eq!(rows.len(), 1, "{}", world.stdout);
    assert_eq!(
        rows[0]["code"].as_str(),
        Some(code.as_str()),
        "{}",
        world.stdout
    );
    assert_eq!(
        rows[0]["file"].as_str(),
        Some(file.as_str()),
        "{}",
        world.stdout
    );
    assert_eq!(
        rows[0]["path"].as_str(),
        Some(path.as_str()),
        "{}",
        world.stdout
    );
}

#[when(expr = "the worked example row is appended to the moves file")]
fn the_worked_example_row_is_appended(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let path = pack.join("rules/verbs.yaml");
    let mut text = fs::read_to_string(&path).unwrap();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&worked_example_row());
    fs::write(path, text).unwrap();
}

#[then(expr = "no tracked file contains a home-directory path")]
fn no_tracked_file_contains_a_home_directory_path(world: &mut World) {
    let hits = absolute_path_hits();
    world.scratch = hits.join("\n");
    assert!(world.scratch.is_empty(), "{}", world.scratch);
}

fn json_stdout(world: &World) -> Vec<Value> {
    let trimmed = world.stdout.trim_end_matches(['\n', '\r']);
    assert_eq!(
        world.stdout,
        format!("{trimmed}\n"),
        "stdout should be one JSON value and a newline"
    );
    let value: Value =
        serde_json::from_str(trimmed).unwrap_or_else(|err| panic!("{err}\n{}", world.stdout));
    value
        .as_array()
        .unwrap_or_else(|| panic!("not an array\n{}", world.stdout))
        .clone()
}

fn json_sort_key(value: &Value) -> (String, Option<u64>, Option<String>) {
    (
        value["file"].as_str().unwrap_or("").to_string(),
        value["line"].as_u64(),
        value["path"].as_str().map(str::to_string),
    )
}

/// The fenced row in the author-pack skill. Appending that text means a
/// broken example fails this scenario.
fn worked_example_row() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.grok/skills/author-pack/SKILL.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read skill: {err}"));
    let marker = "```yaml\n";
    let start = text
        .find(marker)
        .unwrap_or_else(|| panic!("the skill has no yaml example"));
    let rest = &text[start + marker.len()..];
    let end = rest
        .find("\n```")
        .unwrap_or_else(|| panic!("the skill yaml example is not closed"));
    let body = rest[..end].replace('\r', "");
    assert!(
        body.contains("- id: scan_the_street") && body.contains("when:"),
        "the worked example is not a player verb:\n{body}"
    );
    let mut row = body;
    if !row.ends_with('\n') {
        row.push('\n');
    }
    row
}

/// Home-directory paths in every file git tracks.
///
/// A drive letter and a backslash are not enough: Rust string escapes
/// use that shape, so the Windows check requires the home directory
/// after the drive.
fn absolute_path_hits() -> Vec<String> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(&repo)
        .output()
        .unwrap_or_else(|err| panic!("git ls-files: {err}"));
    assert!(
        output.status.success(),
        "git ls-files: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut hits = Vec::new();
    for rel in output.stdout.split(|byte| *byte == 0) {
        if rel.is_empty() {
            continue;
        }
        let rel = std::str::from_utf8(rel).unwrap_or_else(|err| panic!("git path: {err}"));
        let bytes = fs::read(repo.join(rel)).unwrap_or_else(|err| panic!("read {rel}: {err}"));
        let text = String::from_utf8_lossy(&bytes);
        for (index, line) in text.lines().enumerate() {
            if is_scanner_needle(rel, line) {
                continue;
            }
            if has_home_path(line) {
                hits.push(format!("{rel}:{}", index + 1));
            }
        }
    }
    hits
}

/// The needle line in this file. It names both prefixes at once.
fn is_scanner_needle(rel: &str, line: &str) -> bool {
    let rel = rel.replace('\\', "/");
    rel == "crates/spook/tests/steps/check.rs"
        && line.contains(concat!("/Users", "/"))
        && line.contains(concat!("/home", "/"))
}

/// The line below names both prefixes the scan searches for, so the
/// scan skips that line. A home path on any other line still counts.
fn has_home_path(line: &str) -> bool {
    if line.contains("/Users/") || line.contains("/home/") {
        return true;
    }
    has_windows_home(line.as_bytes())
}

fn has_windows_home(bytes: &[u8]) -> bool {
    let users = b"Users\\";
    for index in 0..bytes.len().saturating_sub(1) {
        if !bytes[index].is_ascii_alphabetic() || bytes[index + 1] != b':' {
            continue;
        }
        let mut cursor = index + 2;
        if bytes.get(cursor) != Some(&b'\\') {
            continue;
        }
        while bytes.get(cursor) == Some(&b'\\') {
            cursor += 1;
        }
        let tail = &bytes[cursor..];
        if tail.len() >= users.len() && tail[..users.len()].eq_ignore_ascii_case(users) {
            return true;
        }
    }
    false
}
