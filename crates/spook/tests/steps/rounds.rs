use cucumber::{given, then};
use serde_json::Value;

use crate::support::temp_pack;
use crate::World;

use super::table;

#[given(expr = "a docks table with a GM and a seated player")]
fn a_docks_table_with_a_gm_and_a_seated_player(world: &mut World) {
    let pack = temp_pack("round");
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[then(expr = "tick index is 0")]
fn tick_index_is_0(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).tick_index, 0);
}

#[then(expr = "tick index is 1")]
fn tick_index_is_1(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).tick_index, 1);
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
    world.mark = table::player_len(world);
}

#[then(expr = "the tick log records the player wait")]
fn the_tick_log_records_the_player_wait(world: &mut World) {
    let text = spookery_kernel::api::journal::read_record_line(table::pack(world), "tick-0001")
        .unwrap_or_else(|err| panic!("read tick: {}", err.detail));
    let tick: Value = serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("{err}"));
    let waits = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .count();
    assert_eq!(waits, 1, "{text}");
}
