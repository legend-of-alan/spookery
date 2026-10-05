use std::fs;

use cucumber::{given, then, when};

use crate::support::temp_pack;
use crate::World;

use super::table;

#[given(expr = "a docks table with a seated GM")]
fn a_docks_table_with_a_seated_gm(world: &mut World) {
    let pack = temp_pack("gm-seat");
    let opened =
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    let port = table::free_port();
    let bind = format!("127.0.0.1:{port}");
    table::listen(&pack, &bind, port);
    table::seat_gm();
    crate::with_live(|live| live.opened_now = opened);
    world.pack = Some(pack);
}

#[when(expr = "a second player connects")]
fn a_second_player_connects(world: &mut World) {
    world.scratch = table::second_hello("player:pc_rowan");
}

#[when(expr = "a second GM connects")]
fn a_second_gm_connects(world: &mut World) {
    world.scratch = table::second_hello("gm");
}

#[when(expr = "the operator runs gm")]
fn the_operator_runs_gm(world: &mut World) {
    table::run(world, &["gm"]);
}

#[then(expr = "the second hello is refused with SESSION_FULL")]
fn second_hello_is_refused(world: &mut World) {
    assert_eq!(
        world.scratch,
        "{\"kind\":\"refused\",\"code\":\"SESSION_FULL\"}"
    );
}

#[then(expr = "now.yaml is unchanged since the table opened")]
fn now_unchanged_since_open(world: &mut World) {
    let opened = crate::with_live(|live| live.opened_now.clone());
    let now = fs::read(table::pack(world).join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    assert_eq!(now, opened);
}

#[then(expr = "humans is 1")]
fn humans_is_1(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).presence.humans, 1);
}

#[then(expr = "the present actor is pc_rowan")]
fn the_present_actor_is_pc_rowan(world: &mut World) {
    assert_eq!(
        table::now_of(table::pack(world)).presence.actors_present,
        vec!["pc_rowan".to_string()]
    );
}

#[then(expr = "state_rev is 1")]
fn state_rev_is_1(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).state_rev, 1);
}

#[then(expr = "tick index is 0")]
fn tick_index_is_0(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).tick_index, 0);
}

#[then(expr = "world_time is 1080")]
fn world_time_is_1080(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).world_time, 1080);
}

#[then(expr = "humans is 0")]
fn humans_is_0(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).presence.humans, 0);
}

#[then(expr = "the command exits 1")]
fn the_command_exits_1(world: &mut World) {
    assert_eq!(
        world.code,
        Some(1),
        "stdout:\n{}\nstderr:\n{}",
        world.stdout,
        world.stderr
    );
}

#[then(expr = "stderr is NOT_IN_SPEC")]
fn stderr_is_not_in_spec(world: &mut World) {
    assert_eq!(world.stderr.trim(), "NOT_IN_SPEC", "{}", world.stdout);
}

#[then(expr = "stdout is empty")]
fn stdout_is_empty(world: &mut World) {
    assert!(world.stdout.is_empty(), "{}", world.stdout);
}
