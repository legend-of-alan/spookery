use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
    spookery_kernel::api::canon::pack_rev(&root).unwrap()
}

fn studio_matches(pack: &std::path::Path, revs: &[u64]) {
    use spookery_kernel::api::journal::{emit_entry, read_entries, JournalEntry};
    let studio = fs::read_to_string(pack.join("state/journal/studio.jsonl")).unwrap();
    let entries = read_entries(pack).unwrap_or_else(|err| panic!("{}", err.detail));
    let mut expected = String::new();
    for entry in &entries {
        let JournalEntry::Migrate(record) = entry else {
            continue;
        };
        if revs.contains(&record.state_rev_after) {
            expected.push_str(&emit_entry(entry));
        }
    }
    assert_eq!(studio, expected);
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
        !spookery_kernel::api::journal::list_tick_keys(&pack).is_empty(),
        "no tick committed"
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

#[then(expr = "stderr is SCHEMA")]
fn stderr_is_schema(world: &mut World) {
    assert_eq!(world.stderr.trim(), "SCHEMA", "{}", world.stdout);
}

#[then(expr = "stderr names PACK_REV_MISMATCH")]
fn stderr_names_pack_rev_mismatch(world: &mut World) {
    assert!(
        world.stderr.contains("PACK_REV_MISMATCH"),
        "{}",
        world.stderr
    );
}

#[then(expr = "the tick writes nothing")]
fn the_tick_writes_nothing(world: &mut World) {
    let pack = table::pack(world);
    assert_eq!(
        fs::read(pack.join("state/save.yaml")).unwrap(),
        world.now_bytes
    );
    assert!(!pack.join("state/.txn").exists());
    assert_eq!(
        state_tree(pack),
        world.state_files,
        "the tick wrote under state/"
    );
}

#[then(expr = "spook check reports PACK_REV_MISMATCH")]
fn spook_check_reports_pack_rev_mismatch(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["check", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    assert!(
        world.stderr.contains("PACK_REV_MISMATCH"),
        "{}",
        world.stderr
    );
}

#[then(expr = "the host refuses to start with PACK_REV_MISMATCH")]
fn the_host_refuses_to_start(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let arg = support::pack_arg(&pack);
    let mut child = Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["run", &arg, "--addr", "127.0.0.1:0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap_or_else(|err| panic!("wait: {err}")) {
            break status;
        }
        if start.elapsed() > Duration::from_secs(8) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("host stayed up");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap_or_else(|| panic!("stderr was not piped"))
        .read_to_string(&mut stderr)
        .unwrap_or_else(|err| panic!("read stderr: {err}"));
    world.code = status.code();
    world.stderr = stderr;
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    assert!(
        world.stderr.contains("PACK_REV_MISMATCH"),
        "{}",
        world.stderr
    );
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

#[then(expr = "the studio log records two migrates")]
fn the_studio_log_records_two_migrates(world: &mut World) {
    studio_matches(table::pack(world), &[1, 2]);
}

#[then(expr = "migrate keeps the hand edit")]
fn migrate_keeps_the_hand_edit(world: &mut World) {
    let pack = table::pack(world);
    let rev = spookery_kernel::api::canon::pack_rev(pack).unwrap();
    assert_ne!(rev, docks_rev());
    let now = table::now_of(pack);
    assert_eq!(now.pack_rev, rev);
    assert_eq!(now.state_rev, 1);
    assert_eq!(now.world_time, 7);
    assert_eq!(now.tick_index, 0);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "mira_rooms");
    assert!(spookery_kernel::api::journal::list_tick_keys(pack).is_empty());
    studio_matches(pack, &[1]);
}

#[then(expr = "the studio log is absent")]
fn the_studio_log_is_absent(world: &mut World) {
    assert!(!table::pack(world)
        .join("state/journal/studio.jsonl")
        .exists());
}
