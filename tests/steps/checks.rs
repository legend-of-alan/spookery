use cucumber::{then, when};

use crate::World;

use super::table;

#[when(expr = "the player rolls grit")]
fn the_player_rolls_grit(world: &mut World) {
    let (mark, added) = table::player_send("grit", "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the player sees a check success")]
fn the_player_sees_a_check_success(world: &mut World) {
    world.player_added =
        table::wait_player_from(world.mark, "{\"kind\":\"check\",\"success\":true}");
    assert!(
        !world.player_added.contains("\"faces\""),
        "{}",
        world.player_added
    );
}

#[then(expr = "the tick log resolves the check without faces")]
fn the_tick_log_resolves_the_check_without_faces(world: &mut World) {
    let text = std::fs::read_to_string(table::pack(world).join("state/log/tick-0001.jsonl"))
        .unwrap_or_else(|err| panic!("read tick: {err}"));
    assert!(text.contains("resolve_check"), "{text}");
    assert!(!text.contains("faces"), "{text}");
}

#[then(expr = "the GM sees the grit faces")]
fn the_gm_sees_the_grit_faces(_world: &mut World) {
    let text = table::wait_gm("\"faces\"");
    let event = text
        .lines()
        .find(|line| line.contains("\"faces\""))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(event.contains("\"kind\":\"check\""), "{event}");
    assert!(event.contains("\"faces\":["), "{event}");
    assert!(event.contains("\"stat\":1"), "{event}");
    assert!(event.contains("\"target\":0"), "{event}");
}

#[then(expr = "the player sees a check result without faces")]
fn the_player_sees_a_check_result_without_faces(world: &mut World) {
    let added = table::wait_player_from(world.mark, "{\"kind\":\"check\",\"success\":");
    assert!(
        added.contains("{\"kind\":\"check\",\"success\":true}")
            || added.contains("{\"kind\":\"check\",\"success\":false}"),
        "{added}"
    );
    assert!(!added.contains("\"faces\""), "{added}");
}
