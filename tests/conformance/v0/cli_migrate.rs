use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::support::temp_pack;

const PACK_REV: &str = "blake2s:72453320cb4dccfce9d2b857cfec869790c442acfb81f5fa49df974db367a1ff";

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

fn migrate(pack: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["migrate", pack.to_str().unwrap()])
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"))
}

fn evidence(before_pack: &str, after_pack: &str, before_rev: u64, after_rev: u64) -> String {
    format!(
        "{{\"op\":\"migrate\",\"pack_rev_before\":\"{before_pack}\",\"pack_rev_after\":\"{after_pack}\",\"state_rev_before\":{before_rev},\"state_rev_after\":{after_rev}}}\n"
    )
}

#[test]
fn v0_cli_migrate_bumps_state_rev_and_appends_evidence() {
    let pack = temp_pack("fresh");
    let first = migrate(&pack);
    assert!(
        first.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(first.stdout.is_empty());
    assert!(first.stderr.is_empty());
    let second = migrate(&pack);
    assert!(
        second.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(second.stdout.is_empty());

    let shard = spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    let now = shard.now();
    assert_eq!(now.state_rev, 2);
    assert_eq!(now.tick_index, 0);
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(now.pack_rev, PACK_REV);
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "docks_shop");
    assert_eq!(
        now.places
            .get("docks_shop")
            .unwrap()
            .stock
            .get("lamp_oil")
            .copied(),
        Some(4)
    );
    assert!(!pack.join("state/log/tick-0001.jsonl").exists());
    assert!(!pack.join("state/.txn").exists());
    let studio = fs::read_to_string(pack.join("state/log/studio.jsonl")).unwrap();
    assert_eq!(
        studio,
        format!(
            "{}{}",
            evidence(PACK_REV, PACK_REV, 0, 1),
            evidence(PACK_REV, PACK_REV, 1, 2)
        )
    );
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v0_cli_migrate_keeps_a_hand_edit_and_rewrites_pack_rev() {
    let pack = temp_pack("edit");
    let now_path = pack.join("state/now.yaml");
    let mut text = fs::read_to_string(&now_path).unwrap();
    text = text.replace("world_time: 1080", "world_time: 7");
    text = text.replace(
        "place: docks_shop\n    flags: []\n    busy: false\n    known_facts: []\n    sheet:",
        "place: mira_rooms\n    flags: []\n    busy: false\n    known_facts: []\n    sheet:",
    );
    text = text.replace(
        "    actors:\n      - aldric\n      - pc_rowan\n",
        "    actors:\n      - aldric\n",
    );
    text = text.replace(
        "    actors:\n      - mira\n",
        "    actors:\n      - mira\n      - pc_rowan\n",
    );
    fs::write(&now_path, text).unwrap();
    let mut mira = fs::read_to_string(pack.join("cast/mira.md")).unwrap();
    mira.push('\n');
    fs::write(pack.join("cast/mira.md"), mira).unwrap();

    let output = migrate(&pack);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let rev = spookery_kernel::canon::pack_rev(&pack).unwrap();
    assert_ne!(rev, PACK_REV);
    let shard = spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    let now = shard.now();
    assert_eq!(now.pack_rev, rev);
    assert_eq!(now.state_rev, 1);
    assert_eq!(now.world_time, 7);
    assert_eq!(now.tick_index, 0);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "mira_rooms");
    assert!(!pack.join("state/log/tick-0001.jsonl").exists());
    let studio = fs::read_to_string(pack.join("state/log/studio.jsonl")).unwrap();
    assert_eq!(studio, evidence(PACK_REV, &rev, 0, 1));
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v0_cli_migrate_schema_failure_writes_nothing() {
    let pack = temp_pack("schema");
    fs::write(pack.join("state/now.yaml"), "schema: no\n").unwrap();
    let before = files(&pack);
    let output = migrate(&pack);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        (output.status.code(), stderr.trim()),
        (Some(1), "SCHEMA"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(files(&pack), before, "schema failure wrote the pack");
    assert!(!pack.join("state/log/studio.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}
