use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::copy_docks;
use crate::World;

use super::table;

#[given(expr = "a docks table with a GM and a seated player")]
fn a_docks_table_with_a_gm_and_a_seated_player(world: &mut World) {
    let pack = copy_docks("round");
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "the operator runs spook round")]
fn the_operator_runs_spook_round(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["round", pack.to_str().unwrap()]);
}

#[then(expr = "round index is {int}")]
fn round_index_is(world: &mut World, index: u64) {
    assert_eq!(table::now_of(table::pack(world)).round_index, index);
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

#[then(expr = "the round log records the player wait")]
fn the_round_log_records_the_player_wait(world: &mut World) {
    let text =
        spookery_engine::testing::journal::read_record_line(table::pack(world), "round-0001")
            .unwrap_or_else(|err| panic!("read round: {}", err.detail));
    let round: Value = serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("{err}"));
    let waits = round["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .count();
    assert_eq!(waits, 1, "{text}");
}
