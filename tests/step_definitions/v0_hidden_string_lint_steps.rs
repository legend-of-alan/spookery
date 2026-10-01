use std::fs;
use std::path::Path;

use cucumber::{then, when};

use crate::World;

fn append(pack: &Path, rel: &str, text: &str) {
    let path = pack.join(rel);
    let mut body = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {rel}: {err}"));
    body.push_str(text);
    if !text.ends_with('\n') {
        body.push('\n');
    }
    fs::write(&path, body).unwrap_or_else(|err| panic!("write {rel}: {err}"));
}

fn record_hits(w: &mut World, hits: &[spookery_kernel::lint::LintHit]) {
    w.scratch
        .insert("lint_count".to_string(), hits.len().to_string());
    for (index, hit) in hits.iter().enumerate() {
        w.scratch
            .insert(format!("lint_path_{index}"), hit.path.clone());
        w.scratch
            .insert(format!("lint_text_{index}"), hit.text.clone());
    }
}

fn recorded_hits(w: &World) -> Vec<(String, String)> {
    let count: usize = w
        .scratch
        .get("lint_count")
        .unwrap_or_else(|| panic!("lint was not run"))
        .parse()
        .unwrap_or_else(|err| panic!("lint count: {err}"));
    (0..count)
        .map(|index| {
            (
                w.scratch[&format!("lint_path_{index}")].clone(),
                w.scratch[&format!("lint_text_{index}")].clone(),
            )
        })
        .collect()
}

#[when(expr = "{string} is appended to {string}")]
fn append_sentence(w: &mut World, text: String, rel: String) {
    let pack = w.pack.as_ref().unwrap().clone();
    append(&pack, &rel, &text);
}

#[when(expr = "the pack is linted")]
fn lint_pack(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().clone();
    let hits = spookery_kernel::lint::lint(&pack).unwrap_or_else(|err| panic!("lint: {err}"));
    record_hits(w, &hits);
}

#[then(expr = "the lint hits are empty")]
fn lint_hits_empty(w: &mut World) {
    let hits = recorded_hits(w);
    assert!(hits.is_empty(), "lint hits were not empty: {hits:?}");
}

#[then(expr = "a lint hit is path {string} and text {string}")]
fn lint_hit(w: &mut World, path: String, text: String) {
    let hits = recorded_hits(w);
    assert!(
        hits.iter()
            .any(|(hit_path, hit_text)| hit_path == &path && hit_text == &text),
        "missing lint hit path {path} text {text}: {hits:?}"
    );
}
