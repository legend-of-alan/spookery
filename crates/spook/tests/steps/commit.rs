use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;

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

fn migrate_state_revs(pack: &Path) -> Vec<u64> {
    use spookery_kernel::api::journal::{read_entries, JournalEntry};
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

#[then(expr = "the journal has two migrate records, at state_rev 1 and 2")]
fn the_journal_has_two_migrate_records(world: &mut World) {
    assert_eq!(migrate_state_revs(table::pack(world)), [1, 2]);
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
    assert_eq!(migrate_state_revs(pack), [1]);
}

#[given(expr = "a docks table with a GM and {string} seated")]
fn a_docks_table_with_a_gm_and_named_pc(world: &mut World, actor: String) {
    let pack = support::temp_pack("one-commit");
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, &actor);
    world.pack = Some(pack);
}

#[given(expr = "a docks table one tick before a checkpoint")]
fn a_docks_table_one_tick_before_a_checkpoint(world: &mut World) {
    let pack = support::temp_pack("checkpoint-commit");
    let path = pack.join("pack.yaml");
    let mut text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read pack.yaml: {err}"));
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("checkpoint_every: 1\n");
    fs::write(&path, text).unwrap_or_else(|err| panic!("write pack.yaml: {err}"));
    support::stamp_pack_rev(&pack);
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "pc_rowan");
    world.pack = Some(pack);
}

#[when(expr = "the GM queues a move for {string} and commits")]
fn the_gm_queues_a_move_for_and_commits(world: &mut World, actor: String) {
    queue_wait_and_commit(world, &actor);
}

#[when(expr = "the GM queues a move and commits")]
fn the_gm_queues_a_move_and_commits(world: &mut World) {
    queue_wait_and_commit(world, "pc_rowan");
}

#[then(expr = "the journal has two tick records")]
fn the_journal_has_two_tick_records(world: &mut World) {
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    let keys = table::list_tick_keys(table::pack(world));
    assert_eq!(keys.len(), 2, "{keys:?}");
}

#[then(expr = "the commit reply names the bundle path")]
fn the_commit_reply_names_the_bundle_path(world: &mut World) {
    let path = commit_log_path(world);
    assert!(
        path.contains("state/journal/bundle-"),
        "commit reply log path\n{path}"
    );
    assert!(
        table::pack(world).join(&path).is_file(),
        "missing bundle {path}"
    );
}

#[then(expr = "the commit reply's log path ends in {string}")]
fn the_commit_reply_log_path_ends_in(world: &mut World, suffix: String) {
    let path = commit_log_path(world);
    assert!(path.ends_with(&suffix), "{path}");
    assert!(
        table::pack(world).join(&path).is_file(),
        "missing bundle {path}"
    );
}

fn queue_wait_and_commit(world: &mut World, actor: &str) {
    let args = format!(
        r#"{{"actor_id":"{actor}","actor_kind":"player","source":"player","move":"wait"}}"#
    );
    let queued = table::gm_call(world, 4, "intent", &args);
    assert!(queued.contains("\"queued\""), "queue wait\n{queued}");
    let reply = table::gm_call(world, 5, "commit_round", r#"{"include_tick":true}"#);
    world.stdout = reply.clone();
    world.scratch = reply;
}

fn commit_log_path(world: &World) -> String {
    let value: Value = serde_json::from_str(&world.scratch)
        .unwrap_or_else(|err| panic!("commit reply: {err}\n{}", world.scratch));
    value["result"]["structuredContent"]["log_path"]
        .as_str()
        .unwrap_or_else(|| panic!("no log path\n{}", world.scratch))
        .to_string()
}
