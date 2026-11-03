//! A player and a GM at one table, asserted as facts: who, where, which outcome.

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::copy_docks;
use crate::World;

use super::table;

#[given(expr = "rowan is in the shop and a GM is seated")]
fn rowan_with_a_gm(world: &mut World) {
    let pack = copy_docks("session");
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_rowan(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "the player sends {string}")]
fn the_player_sends(world: &mut World, line: String) {
    world.mark = table::player_write(world, &line);
}

#[when(expr = "the player looks")]
fn the_player_looks(world: &mut World) {
    world.mark = table::player_write(world, "look");
}

#[when(expr = "the player closes the seat")]
fn the_player_closes_the_seat(world: &mut World) {
    table::close_actor(world, "pc_rowan");
}

#[when(expr = "the GM says {string} to the table")]
fn the_gm_says(world: &mut World, text: String) {
    let args = serde_json::json!({ "address": "table", "text": text }).to_string();
    table::call_gm_tool(world, "say", &args);
}

#[when(expr = "the GM closes the seat")]
fn the_gm_closes_the_seat(world: &mut World) {
    table::release_gm(world);
}

#[then(expr = "the player sees rowan arrive at {word}")]
fn the_player_sees_rowan_arrive(world: &mut World, place: String) {
    wait_player_frame(world, &format!("a view at {place}"), |frame| {
        frame["kind"] == "view" && frame["viewer"] == "pc_rowan" && frame["place"] == place
    });
}

#[then(expr = "the player hears the GM say {string}")]
fn the_player_hears_the_gm(world: &mut World, text: String) {
    wait_player_frame(world, "the GM's say", |frame| {
        frame["kind"] == "say" && frame["from"] == "gm" && frame["text"] == text
    });
}

#[then(expr = "the player is shown {string} with an exit to {string}")]
fn the_player_is_shown(world: &mut World, place_name: String, to: String) {
    wait_player_frame(world, &format!("a view of {place_name}"), |frame| {
        let exits = frame["exits"].as_array().cloned().unwrap_or_default();
        frame["kind"] == "view"
            && frame["place_name"] == place_name
            && exits.iter().any(|exit| exit["to"] == to)
    });
}

#[then(expr = "the table refuses the player with {word}")]
fn the_player_is_refused(world: &mut World, code: String) {
    wait_player_frame(world, &format!("a {code} refusal"), |frame| {
        frame["kind"] == "refused" && frame["code"] == code
    });
}

#[then(expr = "the GM sees rowan's travel queued")]
fn the_gm_sees_the_travel_queued(world: &mut World) {
    wait_gm_event(world, "rowan's travel queued", |event| {
        event["kind"] == "queued" && event["actor"] == "pc_rowan" && event["move"] == "travel"
    });
}

#[then(expr = "the GM hears {word} say {string}")]
fn the_gm_hears(world: &mut World, actor_id: String, text: String) {
    let from = format!("player:{actor_id}");
    wait_gm_event(world, &format!("{from}'s say"), |event| {
        event["kind"] == "say" && event["from"] == from && event["text"] == text
    });
}

#[then(expr = "the GM sees rowan in {word}")]
fn the_gm_sees_rowan_in(world: &mut World, place: String) {
    let view = table::call_gm_tool(world, "perceive", r#"{"as":"pc_rowan"}"#);
    assert_eq!(view["place"], place, "{view}");
}

#[then(expr = "the GM sees nothing queued")]
fn the_gm_sees_nothing_queued(world: &mut World) {
    let status = table::call_gm_tool(world, "table_status", "{}");
    assert_eq!(status["queued"], serde_json::json!([]), "{status}");
}

#[then(expr = "the GM sees no player seated")]
fn the_gm_sees_no_player_seated(world: &mut World) {
    table::wait_players(world, 0);
}

/// Waits for a message body after `world.mark` that passes `done`.
fn wait_player_frame(world: &World, want: &str, done: impl Fn(&Value) -> bool) {
    table::poll_until(want, || {
        let text = table::player_text(world);
        let frames = parse_lines(text.get(world.mark..).unwrap_or_default());
        if frames.iter().any(|message| done(&message["body"])) {
            Ok(())
        } else {
            Err(frames)
        }
    });
}

/// Waits for a GM event whose body passes `done`.
fn wait_gm_event(world: &mut World, want: &str, done: impl Fn(&Value) -> bool) {
    table::wait_gm_until(world, want, |text| {
        parse_lines(text)
            .iter()
            .any(|line| done(&line["params"]["body"]))
    });
}

/// Each complete JSON line; a line still arriving is skipped.
fn parse_lines(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
