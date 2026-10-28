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
