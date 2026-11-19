use cucumber::{given, then, when};
use spookery_engine::testing::world_of;

use crate::support::copy_pack;
use crate::World;

use super::table;

#[given(expr = "a docks table with a GM and a seated player")]
async fn a_docks_table_with_a_gm_and_a_seated_player(world: &mut World) {
    let pack = copy_pack("docks", "round");
    table::listen(world, &pack).await;
    table::seat_gm(world).await;
    table::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "the operator runs spook round")]
async fn the_operator_runs_spook_round(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["round", pack.to_str().unwrap()]).await;
}

#[then(expr = "round index is {int}")]
fn round_index_is(world: &mut World, index: u64) {
    assert_eq!(world_of(table::pack(world)).round_index, index);
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
    world.mark = table::player_mark(world);
}
