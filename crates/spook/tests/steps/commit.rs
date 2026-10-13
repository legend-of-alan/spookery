use std::fs;
use std::path::PathBuf;

use cucumber::{then, when};

use crate::World;

use super::table;

fn docks_rev() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
    spookery_kernel::canon::pack_rev(&root).unwrap()
}

fn evidence(before_pack: &str, after_pack: &str, before_rev: u64, after_rev: u64) -> String {
    format!(
        "{{\"op\":\"migrate\",\"pack_rev_before\":\"{before_pack}\",\"pack_rev_after\":\"{after_pack}\",\"state_rev_before\":{before_rev},\"state_rev_after\":{after_rev}}}\n"
    )
}

#[when(expr = "a canon file gains a byte")]
fn a_canon_file_gains_a_byte(world: &mut World) {
    let pack = table::pack(world);
    let mira = pack.join("cast/mira.md");
    let mut body = fs::read_to_string(&mira).unwrap();
    body.push('\n');
    fs::write(&mira, body).unwrap();
    world.now_bytes = fs::read(pack.join("state/save.yaml")).unwrap();
}

#[when(expr = "the operator runs tick")]
fn the_operator_runs_tick(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["tick", pack.to_str().unwrap()]);
}

#[when(expr = "the operator runs migrate")]
fn the_operator_runs_migrate(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["migrate", pack.to_str().unwrap()]);
}

#[when(expr = "the operator migrates twice")]
fn the_operator_migrates_twice(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap();
    for _ in 0..2 {
        table::run(world, &["migrate", arg]);
        assert_eq!(world.code, Some(0), "{}", world.stderr);
        assert!(world.stdout.is_empty(), "{}", world.stdout);
        assert!(world.stderr.is_empty(), "{}", world.stderr);
    }
}

#[when(expr = "the save is hand edited and the canon gains a byte")]
fn the_save_is_hand_edited(world: &mut World) {
    let pack = table::pack(world);
    let now_path = pack.join("state/save.yaml");
    let mut text = fs::read_to_string(&now_path).unwrap();
    text = text.replace("world_time: 1080", "world_time: 7");
    text = text.replace(
        "kind: pc\n    place: docks_shop\n    flags: []\n    busy: false\n    meters:\n      grit:\n        base: 1\n        offset: 0\n      rest:\n        base: 3\n        offset: 0\n    inventory: []\n    known_facts: []",
        "kind: pc\n    place: mira_rooms\n    flags: []\n    busy: false\n    meters:\n      grit:\n        base: 1\n        offset: 0\n      rest:\n        base: 3\n        offset: 0\n    inventory: []\n    known_facts: []",
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
}

#[when(expr = "the save is replaced with a broken document")]
fn the_save_is_replaced(world: &mut World) {
    let pack = table::pack(world);
    fs::write(pack.join("state/save.yaml"), "schema: no\n").unwrap();
    world.before = table::files(pack);
}

#[then(expr = "the command exits 0")]
fn the_command_exits_0(world: &mut World) {
    assert_eq!(
        world.code,
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        world.stdout,
        world.stderr
    );
}

#[then(expr = "stderr is SCHEMA")]
fn stderr_is_schema(world: &mut World) {
    assert_eq!(world.stderr.trim(), "SCHEMA", "{}", world.stdout);
}

#[then(expr = "stderr is empty")]
fn stderr_is_empty(world: &mut World) {
    assert!(world.stderr.is_empty(), "{}", world.stderr);
}

#[then(expr = "the tick writes nothing")]
fn the_tick_writes_nothing(world: &mut World) {
    let pack = table::pack(world);
    assert_eq!(
        fs::read(pack.join("state/save.yaml")).unwrap(),
        world.now_bytes
    );
    assert!(!pack.join("state/.txn").exists());
    assert!(!pack.join("state/journal").exists());
}

#[then(expr = "state_rev is 2")]
fn state_rev_is_2(world: &mut World) {
    let now = table::now_of(table::pack(world));
    assert_eq!(now.state_rev, 2);
    assert_eq!(now.pack_rev, docks_rev());
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
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
}

#[then(expr = "the studio log records two migrates")]
fn the_studio_log_records_two_migrates(world: &mut World) {
    let rev = docks_rev();
    let studio = fs::read_to_string(table::pack(world).join("state/journal/studio.jsonl")).unwrap();
    assert_eq!(
        studio,
        format!(
            "{}{}",
            evidence(&rev, &rev, 0, 1),
            evidence(&rev, &rev, 1, 2)
        )
    );
}

#[then(expr = "migrate keeps the hand edit")]
fn migrate_keeps_the_hand_edit(world: &mut World) {
    let pack = table::pack(world);
    let rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    assert_ne!(rev, docks_rev());
    let now = table::now_of(pack);
    assert_eq!(now.pack_rev, rev);
    assert_eq!(now.state_rev, 1);
    assert_eq!(now.world_time, 7);
    assert_eq!(now.tick_index, 0);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "mira_rooms");
    assert!(!pack.join("state/journal/tick-0001.jsonl").exists());
    let studio = fs::read_to_string(pack.join("state/journal/studio.jsonl")).unwrap();
    assert_eq!(studio, evidence(&docks_rev(), &rev, 0, 1));
}

#[then(expr = "the studio log is absent")]
fn the_studio_log_is_absent(world: &mut World) {
    assert!(!table::pack(world)
        .join("state/journal/studio.jsonl")
        .exists());
}
