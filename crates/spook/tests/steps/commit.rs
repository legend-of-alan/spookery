use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};

use crate::support;
use crate::World;

use super::table;

fn state_tree(pack: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let state = pack.join("state");
    if state.is_dir() {
        table::files(&state)
    } else {
        Vec::new()
    }
}

fn docks_rev() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
    spookery_engine::pack_rev(&root).unwrap()
}

fn migrate_state_revs(pack: &Path) -> Vec<u64> {
    use spookery_engine::testing::journal::{read_entries, JournalEntry};
    let entries = read_entries(pack).unwrap_or_else(|err| panic!("{}", err.detail));
    let mut revs = Vec::new();
    for entry in &entries {
        let JournalEntry::Migrate(record) = entry else {
            continue;
        };
        revs.push(record.state_rev_after);
    }
    revs
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

#[given(expr = "one round has committed")]
fn one_round_has_committed(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["tick", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    assert!(
        !spookery_engine::testing::journal::list_round_keys(&pack).is_empty(),
        "no round committed"
    );
}

#[when(expr = "the operator runs tick")]
fn the_operator_runs_tick(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    world.state_files = state_tree(&pack);
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
        "kind: pc\n    place: docks_shop\n    flags: []\n    meters:\n      grit:\n        base: 1\n        offset: 0\n      rest:\n        base: 3\n        offset: 0\n    inventory: {}\n    known_facts: []",
        "kind: pc\n    place: mira_rooms\n    flags: []\n    meters:\n      grit:\n        base: 1\n        offset: 0\n      rest:\n        base: 3\n        offset: 0\n    inventory: {}\n    known_facts: []",
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
    a_canon_file_gains_a_byte(world);
}

#[when(expr = "the pack adds meter {string} with start {int}")]
fn the_pack_adds_meter(world: &mut World, id: String, start: i64) {
    let path = table::pack(world).join("rules/meters.yaml");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!(
        "  - id: {id}\n    min: 0\n    base_cap: 10\n    max: 10\n    start: {start}\n"
    ));
    fs::write(&path, text).unwrap();
}

#[then(expr = "every actor has {string} at {int}")]
fn every_actor_has_meter_at(world: &mut World, id: String, base: i64) {
    let now = table::now_of(table::pack(world));
    for (actor, live) in &now.actors {
        let got = live.meters().get(&id).map(|meter| meter.base);
        assert_eq!(got, Some(base), "{actor} {id}");
    }
}

#[when(expr = "the save is replaced with a broken document")]
fn the_save_is_replaced(world: &mut World) {
    let pack = table::pack(world);
    fs::write(pack.join("state/save.yaml"), "schema: no\n").unwrap();
    world.before = table::files(pack);
}

#[then(expr = "stderr names {word}")]
fn stderr_names(world: &mut World, code: String) {
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "the round writes nothing")]
fn the_round_writes_nothing(world: &mut World) {
    let pack = table::pack(world);
    assert_eq!(
        fs::read(pack.join("state/save.yaml")).unwrap(),
        world.now_bytes
    );
    assert!(!pack.join("state/.txn").exists());
    assert_eq!(
        state_tree(pack),
        world.state_files,
        "the round wrote under state/"
    );
}

#[when(expr = "the save drops meter {string}")]
fn the_save_drops_meter(world: &mut World, id: String) {
    let path = table::pack(world).join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let needle = format!("      {id}:");
    let mut out = String::new();
    let mut skip = 0;
    let mut found = false;
    for line in text.lines() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if line == needle {
            found = true;
            skip = 2;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    assert!(found, "save has no meter {id}");
    fs::write(&path, out).unwrap();
}

#[then(expr = "spook check reports {word}")]
fn spook_check_reports(world: &mut World, code: String) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["check", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "the host refuses to start with {word}")]
fn the_host_refuses_to_start(world: &mut World, code: String) {
    let pack = table::pack(world).to_path_buf();
    let arg = support::pack_arg(&pack);
    let mut proc = support::spawn(&["run", &arg, "--addr", "127.0.0.1:0"]);
    let start = Instant::now();
    let (status, stderr) = loop {
        let status = proc
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("wait: {err}"));
        let stderr = support::text_of(&proc.stderr);
        if let Some(status) = status {
            if !stderr.is_empty() {
                break (status, stderr);
            }
        }
        if start.elapsed() > Duration::from_secs(8) {
            panic!("host stayed up\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    world.code = status.code();
    world.stderr = stderr;
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "spook verify passes")]
fn spook_verify_passes(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", pack.to_str().unwrap()]);
    assert_eq!(
        world.code,
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        world.stdout,
        world.stderr
    );
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
            .items
            .get("lamp_oil")
            .copied(),
        Some(4u32)
    );
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
}

#[then(expr = "the journal has two migrate records, at state_rev 1 and 2")]
fn the_journal_has_two_migrate_records(world: &mut World) {
    assert_eq!(migrate_state_revs(table::pack(world)), [1, 2]);
}

#[then(expr = "migrate keeps the hand edit")]
fn migrate_keeps_the_hand_edit(world: &mut World) {
    let pack = table::pack(world);
    let rev = spookery_engine::pack_rev(pack).unwrap();
    assert_ne!(rev, docks_rev());
    let now = table::now_of(pack);
    assert_eq!(now.pack_rev, rev);
    assert_eq!(now.state_rev, 1);
    assert_eq!(now.world_time, 7);
    assert_eq!(now.round_index, 0);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "mira_rooms");
    assert!(spookery_engine::testing::journal::list_round_keys(pack).is_empty());
    assert_eq!(migrate_state_revs(pack), [1]);
}
