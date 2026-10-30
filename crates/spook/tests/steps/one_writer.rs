use cucumber::{given, then, when};

use crate::support::copy_docks;
use crate::World;

use super::table;

fn read_state_files(world: &World) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    table::read_files(&table::pack(world).join("state"))
}

#[given(expr = "spook run is serving a docks table")]
fn spook_run_is_serving_a_docks_table(world: &mut World) {
    let pack = copy_docks("one-writer");
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
    world.state_files = read_state_files(world);
    let pack = table::pack(world).to_path_buf();
    table::run(world, &[&command, pack.to_str().unwrap()]);
}

#[given(expr = "spook round was refused")]
fn spook_round_was_refused(world: &mut World) {
    the_host_runs_spook_on_that_pack(world, "round".to_string());
    spook_exits_non_zero_with(world, "round".to_string(), "COMMIT_CONFLICT".to_string());
}

#[then(expr = "spook {word} exits non-zero with {word}")]
fn spook_exits_non_zero_with(world: &mut World, command: String, code: String) {
    assert_ne!(world.code, Some(0), "spook {command} exited 0");
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "the state is unchanged")]
fn the_state_is_unchanged(world: &mut World) {
    assert_eq!(read_state_files(world), world.state_files, "state/ changed");
}

#[when(expr = "a seated player commits a move")]
fn a_seated_player_commits_a_move(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::seat_rowan(world, &pack);
    table::actor_send(world, "pc_rowan", "wait", "\"kind\":");
    table::wait_committed_wait(&pack, "pc_rowan", 0);
}

#[when(expr = "the host runs spook {word} on a missing pack")]
fn the_host_runs_spook_on_a_missing_pack(world: &mut World, command: String) {
    let pack = copy_docks("missing-pack");
    let missing = pack.join("no-such-pack");
    world.pack = Some(pack);
    table::run(world, &[&command, missing.to_str().unwrap()]);
}
