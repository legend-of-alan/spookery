use std::fs;
use std::path::{Path, PathBuf};

use spookery_kernel::Reason;
use spookery_mcp::tick_tools::{perceive, PerceiveIn};

use super::support::copy_dir;

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

fn call(
    shard: &spookery_kernel::store::Shard,
    as_viewer: Option<&str>,
    focus: Option<&str>,
    budget_tokens: Option<u64>,
) -> Result<spookery_kernel::perceive::Perceive, Reason> {
    perceive(
        shard,
        &PerceiveIn {
            as_viewer: as_viewer.map(str::to_string),
            focus: focus.map(str::to_string),
            budget_tokens,
        },
    )
}

#[test]
fn v0_t09_tool_scope() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-tick09-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);

    let shard =
        spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let before = files(&pack);

    let missing = call(&shard, None, None, None).expect_err("missing as");
    assert_eq!(missing.token(), "UNSCOPED_PERCEIVE");

    for as_viewer in [
        "",
        "gm",
        "system",
        "system:clock",
        "npc:",
        "player:pc_rowan",
    ] {
        let err = call(&shard, Some(as_viewer), None, None).expect_err(as_viewer);
        assert_eq!(err.token(), "UNSCOPED_PERCEIVE", "{as_viewer}");
    }

    let focus = call(&shard, Some("npc:mira"), Some("stock"), None).expect_err("focus");
    assert_eq!(focus.token(), "SCHEMA");

    let kernel =
        spookery_kernel::perceive(&shard, "npc:mira").unwrap_or_else(|err| panic!("{err}"));
    for focus in [None, Some("place"), Some("actor")] {
        let view =
            call(&shard, Some("npc:mira"), focus, None).unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(view, kernel);
    }
    let default_budget =
        call(&shard, Some("npc:mira"), None, Some(800)).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(default_budget, kernel);

    let capped =
        call(&shard, Some("npc:mira"), None, Some(0)).unwrap_or_else(|err| panic!("{err}"));
    assert!(kernel.known_facts.iter().any(|fact| !fact.body.is_empty()));
    assert_eq!(capped.known_facts.len(), kernel.known_facts.len());
    assert!(capped.known_facts.iter().all(|fact| fact.body.is_empty()));
    let ledger = capped
        .known_facts
        .iter()
        .find(|fact| fact.id == "fact_ledger_short")
        .unwrap_or_else(|| panic!("mira keeps fact_ledger_short"));
    assert_eq!(ledger.text, "The lamp-oil ledger is short.");
    assert!(!capped
        .unknown_mentioned
        .iter()
        .any(|id| id == "fact_ledger_short"));
    assert!(capped
        .unknown_mentioned
        .iter()
        .any(|id| id == "fact_room_lamp"));

    assert_eq!(files(&pack), before);
    let _ = fs::remove_dir_all(&pack);
}
