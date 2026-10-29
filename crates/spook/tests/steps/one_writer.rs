use cucumber::{given, then, when};

use crate::support::temp_pack;
use crate::World;

use super::table;

fn state_files(world: &World) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    table::files(&table::pack(world).join("state"))
}

#[given(expr = "spook run is serving a docks table")]
fn spook_run_is_serving_a_docks_table(world: &mut World) {
    let pack = temp_pack("one-writer");
    table::listen(world, &pack);
    world.pack = Some(pack);
}

#[given(expr = "a commit folder is left in the pack")]
fn a_commit_folder_is_left_in_the_pack(world: &mut World) {
    let txn = table::pack(world).join("state/.txn/prev");
    std::fs::create_dir_all(&txn).unwrap();
    std::fs::write(txn.join("save.yaml"), b"not-yaml\n").unwrap();
}

#[when(expr = "the host runs spook {word} on that pack")]
fn the_host_runs_spook_on_that_pack(world: &mut World, command: String) {
    world.state_files = state_files(world);
    let pack = table::pack(world).to_path_buf();
    table::run(world, &[&command, pack.to_str().unwrap()]);
}

#[given(expr = "spook tick was refused")]
fn spook_tick_was_refused(world: &mut World) {
    the_host_runs_spook_on_that_pack(world, "tick".to_string());
    spook_exits_non_zero_with(world, "tick".to_string(), "COMMIT_CONFLICT".to_string());
}

#[then(expr = "spook {word} exits non-zero with {word}")]
fn spook_exits_non_zero_with(world: &mut World, command: String, code: String) {
    assert_ne!(world.code, Some(0), "spook {command} exited 0");
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "the journal is unchanged")]
fn the_journal_is_unchanged(world: &mut World) {
    assert_eq!(state_files(world), world.state_files, "state/ changed");
}

#[when(expr = "a seated player commits a move")]
fn a_seated_player_commits_a_move(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::seat_player(world, &pack);
    table::actor_send(world, "pc_rowan", "wait", "\"kind\":");
    table::expect_wait_from(&pack, "pc_rowan", 0);
}

#[when(expr = "the host runs spook {word} on a missing pack")]
fn the_host_runs_spook_on_a_missing_pack(world: &mut World, command: String) {
    let pack = temp_pack("missing-pack");
    let missing = pack.join("no-such-pack");
    world.pack = Some(pack);
    table::run(world, &[&command, missing.to_str().unwrap()]);
}

#[given(expr = "a writer holds the pack lock")]
fn a_writer_holds_the_pack_lock(world: &mut World) {
    world.lock = Some(spookery_engine::lock_pack(table::pack(world)).unwrap());
}

#[given(expr = "a second handle to the lock file stays open, like a child that has not exec'd yet")]
fn a_second_handle_stays_open(world: &mut World) {
    let lock = world.lock.as_ref().unwrap();
    world.lock_copy = Some(spookery_engine::testing::copy_lock_handle(lock));
}

#[when(expr = "the writer drops its lock")]
fn the_writer_drops_its_lock(world: &mut World) {
    world.lock = None;
}

#[then(expr = "a new writer can lock the pack")]
fn a_new_writer_can_lock_the_pack(world: &mut World) {
    if let Err(err) = spookery_engine::lock_pack(table::pack(world)) {
        panic!("lock_pack: {err}");
    }
}
