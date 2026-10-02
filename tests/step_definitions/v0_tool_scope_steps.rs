use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then};

use crate::World;

fn open(w: &World) -> spookery_kernel::store::Shard {
    spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("open_shard: {err}"))
}

fn call(
    shard: &spookery_kernel::store::Shard,
    as_viewer: Option<&str>,
    focus: Option<&str>,
    budget_tokens: Option<u64>,
) -> Result<spookery_kernel::perceive::Perceive, spookery_kernel::Reason> {
    spookery_mcp::tick_tools::perceive(
        shard,
        &spookery_mcp::tick_tools::PerceiveIn {
            as_viewer: as_viewer.map(str::to_string),
            focus: focus.map(str::to_string),
            budget_tokens,
        },
    )
}

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
    format!("scope-file:{}", path.display())
}

#[given(expr = "the pack files are recorded")]
fn record_files(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().clone();
    let _shard = open(w);
    let listed = files(&pack);
    w.scratch
        .insert("scope-file-count".to_string(), listed.len().to_string());
    for (path, bytes) in listed {
        w.scratch.insert(file_key(&path), hex(&bytes));
    }
}

#[then(expr = "tick-profile perceive with no as is {word}")]
fn perceive_no_as(w: &mut World, code: String) {
    let shard = open(w);
    let err = call(&shard, None, None, None).expect_err("missing as");
    assert_eq!(err.token(), code);
}

#[then(expr = "tick-profile perceive as {string} is {word}")]
fn perceive_as_rejected(w: &mut World, as_viewer: String, code: String) {
    let shard = open(w);
    let err = call(&shard, Some(&as_viewer), None, None).expect_err(&as_viewer);
    assert_eq!(err.token(), code, "{as_viewer}");
}

#[then(expr = "tick-profile perceive as {string} with focus {string} is {word}")]
fn perceive_focus_rejected(w: &mut World, as_viewer: String, focus: String, code: String) {
    let shard = open(w);
    let err = call(&shard, Some(&as_viewer), Some(&focus), None).expect_err(&focus);
    assert_eq!(err.token(), code);
}

#[then(expr = "tick-profile perceive as {string} matches kernel perceive")]
fn perceive_matches_kernel(w: &mut World, as_viewer: String) {
    let shard = open(w);
    let kernel =
        spookery_kernel::perceive(&shard, &as_viewer).unwrap_or_else(|err| panic!("{err}"));
    let view = call(&shard, Some(&as_viewer), None, None).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(view, kernel);
}

#[then(expr = "tick-profile perceive as {string} with focus {string} matches kernel perceive")]
fn perceive_focus_matches_kernel(w: &mut World, as_viewer: String, focus: String) {
    let shard = open(w);
    let kernel =
        spookery_kernel::perceive(&shard, &as_viewer).unwrap_or_else(|err| panic!("{err}"));
    let view =
        call(&shard, Some(&as_viewer), Some(&focus), None).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(view, kernel);
}

#[then(expr = "tick-profile perceive as {string} with budget {int} matches kernel perceive")]
fn perceive_budget_matches_kernel(w: &mut World, as_viewer: String, budget: u64) {
    let shard = open(w);
    let kernel =
        spookery_kernel::perceive(&shard, &as_viewer).unwrap_or_else(|err| panic!("{err}"));
    let view =
        call(&shard, Some(&as_viewer), None, Some(budget)).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(view, kernel);
}

#[then(expr = "kernel perceive for {string} still has a fact body")]
fn kernel_still_has_a_fact_body(w: &mut World, as_viewer: String) {
    let shard = open(w);
    let kernel =
        spookery_kernel::perceive(&shard, &as_viewer).unwrap_or_else(|err| panic!("{err}"));
    assert!(kernel.known_facts.iter().any(|fact| !fact.body.is_empty()));
}

#[then(
    expr = "tick-profile perceive as {string} with budget {int} has the kernel fact count and empty fact bodies"
)]
fn perceive_budget_strips_bodies(w: &mut World, as_viewer: String, budget: u64) {
    let shard = open(w);
    let kernel =
        spookery_kernel::perceive(&shard, &as_viewer).unwrap_or_else(|err| panic!("{err}"));
    let capped =
        call(&shard, Some(&as_viewer), None, Some(budget)).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(capped.known_facts.len(), kernel.known_facts.len());
    assert!(capped.known_facts.iter().all(|fact| fact.body.is_empty()));
}

#[then(
    expr = "tick-profile perceive as {string} with budget {int} keeps fact {string} text {string}"
)]
fn perceive_budget_keeps_fact_text(
    w: &mut World,
    as_viewer: String,
    budget: u64,
    id: String,
    text: String,
) {
    let shard = open(w);
    let capped =
        call(&shard, Some(&as_viewer), None, Some(budget)).unwrap_or_else(|err| panic!("{err}"));
    let fact = capped
        .known_facts
        .iter()
        .find(|fact| fact.id == id)
        .unwrap_or_else(|| panic!("mira keeps {id}"));
    assert_eq!(fact.text, text);
}

#[then(expr = "tick-profile perceive as {string} with budget {int} does not list unknown {string}")]
fn perceive_budget_omits_unknown(w: &mut World, as_viewer: String, budget: u64, id: String) {
    let shard = open(w);
    let capped =
        call(&shard, Some(&as_viewer), None, Some(budget)).unwrap_or_else(|err| panic!("{err}"));
    assert!(!capped.unknown_mentioned.iter().any(|item| item == &id));
}

#[then(expr = "tick-profile perceive as {string} with budget {int} lists unknown {string}")]
fn perceive_budget_lists_unknown(w: &mut World, as_viewer: String, budget: u64, id: String) {
    let shard = open(w);
    let capped =
        call(&shard, Some(&as_viewer), None, Some(budget)).unwrap_or_else(|err| panic!("{err}"));
    assert!(capped.unknown_mentioned.iter().any(|item| item == &id));
}

#[then(expr = "the pack files are unchanged")]
fn pack_files_unchanged(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let listed = files(pack);
    let count: usize = w.scratch.get("scope-file-count").unwrap().parse().unwrap();
    assert_eq!(listed.len(), count);
    for (path, bytes) in listed {
        let saved = w
            .scratch
            .get(&file_key(&path))
            .unwrap_or_else(|| panic!("missing {}", path.display()));
        assert_eq!(saved, &hex(&bytes), "{}", path.display());
    }
}
