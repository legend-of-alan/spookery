use std::fs;

use cucumber::{then, when};
use serde_json::Value;

use crate::World;

use super::table;

#[when(expr = "the operator runs verify")]
fn the_operator_runs_verify(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", pack.to_str().unwrap()]);
}

#[when(expr = "the operator runs replay")]
fn the_operator_runs_replay(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["replay", pack.to_str().unwrap()]);
}

#[when(expr = "the operator runs {int} ticks")]
fn the_operator_runs_n_ticks(world: &mut World, count: usize) {
    let pack = table::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap().to_string();
    for _ in 0..count {
        table::run(world, &["tick", &arg]);
        assert_eq!(world.code, Some(0), "{}", world.stderr);
    }
}

#[when(expr = "two players play, a GM commits one round, and the players leave")]
fn two_players_play_a_round(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let port = table::free_port();
    table::listen(&pack, &format!("127.0.0.1:{port}"), port);
    table::seat_actor(&pack, "pc_rowan");
    table::seat_actor(&pack, "pc_harbormouse");
    let _ = table::actor_send("pc_rowan", "wait", "\"kind\":");
    let _ = table::actor_send("pc_harbormouse", "wait", "\"kind\":");
    table::seat_gm();
    let _ = table::actor_send("pc_rowan", "wait", "\"kind\":");
    let _ = table::gm_call(7, "commit_turn", r#"{"include_tick":true}"#);
    table::close_actor("pc_harbormouse");
    table::wait_humans(&pack, 1);
    table::close_actor("pc_rowan");
    table::wait_humans(&pack, 0);
    table::stop_server();
}

#[when(expr = "a seated player's server is killed and restarted")]
fn a_seated_player_is_killed_and_restarted(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let port = table::free_port();
    table::listen(&pack, &format!("127.0.0.1:{port}"), port);
    table::seat_actor(&pack, "pc_rowan");
    table::wait_humans(&pack, 1);
    table::stop_server();
    let port = table::free_port();
    table::listen(&pack, &format!("127.0.0.1:{port}"), port);
    table::wait_humans(&pack, 0);
    table::stop_server();
}

#[when(expr = "a player sits and the server stops")]
fn a_player_sits_and_the_server_stops(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let port = table::free_port();
    table::listen(&pack, &format!("127.0.0.1:{port}"), port);
    table::seat_actor(&pack, "pc_rowan");
    table::wait_humans(&pack, 1);
    table::stop_server();
}

#[when(expr = "one intent in a tick record is changed")]
fn one_intent_in_a_tick_record_is_changed(world: &mut World) {
    let path = table::pack(world).join("state/log/tick-0001.jsonl");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read tick: {err}"));
    let mut tick: Value =
        serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("json: {err}"));
    let intents = tick["intents"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("no intents\n{text}"));
    assert!(!intents.is_empty(), "{text}");
    intents[0]["move"] = Value::String("not_a_real_move".to_string());
    let rev = tick["state_rev_after"]
        .as_u64()
        .unwrap_or_else(|| panic!("{text}"));
    let index = tick["tick_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("{text}"));
    world.scratch = format!("state_rev {rev} tick {index}");
    fs::write(&path, serde_json::to_string(&tick).unwrap()).unwrap();
}

#[when(expr = "one presence record is deleted")]
fn one_presence_record_is_deleted(world: &mut World) {
    let path = table::pack(world).join("state/log/presence-0001.json");
    assert!(path.is_file(), "no presence record at {}", path.display());
    fs::remove_file(&path).unwrap_or_else(|err| panic!("remove presence: {err}"));
    world.scratch = "state_rev 1".to_string();
}

#[when(expr = "the pack bytes are remembered")]
fn the_pack_bytes_are_remembered(world: &mut World) {
    world.before = table::files(table::pack(world));
}

#[then(expr = "stdout says the save was verified")]
fn stdout_says_the_save_was_verified(world: &mut World) {
    let rev = table::now_of(table::pack(world)).state_rev;
    assert!(
        world.stdout.contains("verified ") && world.stdout.contains("records, state_rev "),
        "{}",
        world.stdout
    );
    assert!(
        world.stdout.contains(&format!("state_rev {rev}")),
        "stdout {} live {rev}",
        world.stdout
    );
}

#[then(expr = "stdout shows the final state_rev")]
fn stdout_shows_the_final_state_rev(world: &mut World) {
    let rev = table::now_of(table::pack(world)).state_rev;
    assert!(
        world.stdout.contains(&format!("state_rev {rev}")),
        "{}",
        world.stdout
    );
    assert!(world.stdout.contains("records"), "{}", world.stdout);
}

#[then(expr = "stderr names that record")]
fn stderr_names_that_record(world: &mut World) {
    assert!(
        world.stderr.contains(world.scratch.trim()),
        "missing {}\n{}",
        world.scratch,
        world.stderr
    );
}

#[then(expr = "stderr names the first differing save line")]
fn stderr_names_the_first_differing_save_line(world: &mut World) {
    assert!(world.stderr.contains("now.yaml:"), "{}", world.stderr);
}
