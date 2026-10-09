use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::temp_pack;
use crate::World;

use super::table;

#[given(expr = "a docks table with a GM and a seated player")]
fn a_docks_table_with_a_gm_and_a_seated_player(world: &mut World) {
    let pack = temp_pack("round");
    let port = table::free_port();
    let bind = format!("127.0.0.1:{port}");
    table::listen(&pack, &bind, port);
    table::seat_gm();
    table::seat_player(&pack);
    world.pack = Some(pack);
}

#[when(expr = "the player waits")]
fn the_player_waits(world: &mut World) {
    let (mark, added) = table::player_send("wait", "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the GM commits the round")]
fn the_gm_commits_the_round(_world: &mut World) {
    let _ = table::gm_call(2, "commit_turn", r#"{"include_tick":true}"#);
}

#[then(expr = "world_time is 1085")]
fn world_time_is_1085(world: &mut World) {
    table::wait_world_time(table::pack(world), 1085);
}

#[then(expr = "tick index is 1")]
fn tick_index_is_1(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).tick_index, 1);
}

#[then(expr = "a tick file was written")]
fn a_tick_file_was_written(world: &mut World) {
    assert_eq!(
        table::tick_names(table::pack(world)),
        vec!["tick-0001.jsonl".to_string()]
    );
}

#[then(expr = "the player line is queued")]
fn the_player_line_is_queued(world: &mut World) {
    assert!(
        world.player_added.contains("\"kind\":\"queued\""),
        "{}",
        world.player_added
    );
    assert!(
        !world.player_added.contains("\"kind\":\"check\""),
        "{}",
        world.player_added
    );
    world.mark = table::player_len();
}

#[then(expr = "the tick log records the player wait")]
fn the_tick_log_records_the_player_wait(world: &mut World) {
    let text = std::fs::read_to_string(table::pack(world).join("state/log/tick-0001.jsonl"))
        .unwrap_or_else(|err| panic!("read tick: {err}"));
    let tick: Value = serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("{err}"));
    let waits = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .count();
    assert_eq!(waits, 1, "{text}");
}

