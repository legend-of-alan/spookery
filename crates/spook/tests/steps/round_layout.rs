use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then};

use crate::support;
use crate::World;

const PUBLIC: &[&str] = &[
    "ReplayError",
    "ReplayReport",
    "ReplayStep",
    "RoundCause",
    "VerifyError",
    "VerifyReport",
    "advance_while_busy",
    "catch_up",
    "commit_moves",
    "commit_moves_at",
    "from_journal",
    "migrate_pack",
    "plan_catchup",
    "play",
    "play_pack_once",
    "replay_states",
    "resume",
    "verify_pack",
    "verify_parallel",
    "verify_quick",
];

#[given(expr = "the round crate after the move")]
fn the_round_crate_after_the_move(_world: &mut World) {
    let round = repo_root().join("crates/round");
    assert_eq!(
        entries(&round),
        names(&[
            "Cargo.toml",
            "lib.rs",
            "play.rs",
            "prop",
            "replay.rs",
            "seed.rs",
            "stage",
            "state.rs",
        ]),
        "round's top level should be the driver, replay, and helpers"
    );
    assert_eq!(
        entries(&round.join("stage")),
        names(&[
            "advance.rs",
            "apply.rs",
            "complete.rs",
            "gather.rs",
            "mod.rs",
            "observe.rs",
            "record.rs",
            "resolve.rs",
            "triggers.rs",
        ]),
        "the eight stages live under round/stage"
    );
    assert_eq!(
        entries(&round.join("prop")),
        names(&["mod.rs", "play.rs", "replay.rs", "rerun.rs", "triggers.rs",]),
        "property tests live under round/prop"
    );
    let record = fs::read_to_string(round.join("stage/record.rs"))
        .unwrap_or_else(|err| panic!("read record stage: {err}"));
    assert!(
        record.contains("struct RoundLine") && record.contains("fn render_round_line"),
        "the round log is not folded into the record stage"
    );
    let mut files = Vec::new();
    rust_files(&round, &mut files);
    for path in files {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        assert!(
            !text.contains("round::round") && !text.contains("round_seed::round_seed"),
            "{} still stutters",
            path.display()
        );
        for line in text.lines() {
            let trimmed = line.trim();
            assert!(
                trimmed != "mod round;" && trimmed != "mod round_seed;",
                "{} still declares {trimmed}",
                path.display()
            );
        }
    }
}

#[then(expr = "spook builds without import changes")]
fn spook_builds_without_import_changes(_world: &mut World) {
    let root = repo_root();
    let lib = fs::read_to_string(root.join("crates/round/lib.rs"))
        .unwrap_or_else(|err| panic!("read round lib: {err}"));
    let exported = exported_names(&lib);
    let expected = names(PUBLIC);
    assert_eq!(exported, expected, "round's public exports changed");
    let mut used = BTreeSet::new();
    let mut files = Vec::new();
    rust_files(&root.join("crates/spook"), &mut files);
    let needle = concat!("spookery_", "round");
    for path in files {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        for name in crate_names(&text, needle) {
            assert!(
                expected.contains(&name),
                "{} imports {name}, which is not a public round export",
                path.display()
            );
            used.insert(name);
        }
    }
    assert!(
        used.contains("play") && used.contains("verify_pack") && used.contains("plan_catchup"),
        "spook no longer uses the public round API: {used:?}"
    );
}

#[then(expr = "spook verify passes on the existing feature packs")]
fn spook_verify_passes_on_the_existing_feature_packs(_world: &mut World) {
    for name in ["oneshot-docks", "sample-d20"] {
        let pack = crate::support::copy_pack(name, &format!("layout-{name}"));
        let output = support::spook(&["verify", &pack.to_string_lossy()]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_dir_all(&pack);
        assert!(
            output.status.success(),
            "{name} verify failed\n{stdout}{stderr}"
        );
    }
}

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

fn entries(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn exported_names(lib: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for chunk in lib.split("pub use ").skip(1) {
        let body = chunk.split(';').next().unwrap_or("").trim();
        if let Some(open) = body.find('{') {
            let inside = body[open + 1..].split('}').next().unwrap_or("");
            for item in inside.split(',') {
                if let Some(name) = public_name(item) {
                    names.insert(name);
                }
            }
        } else if let Some(name) = body.rsplit("::").next() {
            let name = name.trim();
            if !name.is_empty() {
                names.insert(name.to_string());
            }
        }
    }
    names
}

fn public_name(item: &str) -> Option<String> {
    let mut words = item.split_whitespace();
    let first = words.next()?;
    let name = if words.next() == Some("as") {
        words.next().unwrap_or(first)
    } else {
        first
    };
    let name = name.trim_matches(|ch: char| ch == ',' || ch == '\n');
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn crate_names(text: &str, needle: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find(needle) {
        let after = &rest[idx + needle.len()..];
        if let Some(body) = after.strip_prefix("::{") {
            let inside = body.split('}').next().unwrap_or("");
            for item in inside.split(',') {
                if let Some(name) = public_name(item) {
                    names.push(name);
                }
            }
            rest = body;
        } else if let Some(body) = after.strip_prefix("::") {
            let name: String = body
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect();
            if !name.is_empty() {
                names.push(name);
            }
            rest = body;
        } else {
            rest = after;
        }
    }
    names
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display())) {
        let path = entry
            .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
            .path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
