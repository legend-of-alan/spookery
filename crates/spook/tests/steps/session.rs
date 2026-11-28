//! A player and a GM at one host, asserted as facts: who, where, which outcome.

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::copy_pack;
use crate::World;

use super::host;

#[given(expr = "rowan is in the shop and a GM is seated")]
async fn rowan_with_a_gm(world: &mut World) {
    let pack = copy_pack("docks", "session");
    host::listen(world, &pack).await;
    host::seat_gm(world).await;
    host::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "the player sends {string}")]
fn the_player_sends(world: &mut World, line: String) {
    world.mark = host::player_write(world, &line);
}

#[when(expr = "the player looks")]
fn the_player_looks(world: &mut World) {
    world.mark = host::player_write(world, "look");
}

#[when(expr = "the player closes the seat")]
async fn the_player_closes_the_seat(world: &mut World) {
    host::close_actor(world, "rowan").await;
}

#[when(expr = "the GM says {string} to the table")]
async fn the_gm_says(world: &mut World, text: String) {
    let args = serde_json::json!({ "to": "table", "text": text }).to_string();
    host::call_gm_tool(world, "say", &args).await;
}

#[when(expr = "the GM closes the seat")]
async fn the_gm_closes_the_seat(world: &mut World) {
    host::release_gm(world).await;
}

#[then(expr = "the player sees rowan arrive at {word}")]
async fn the_player_sees_rowan_arrive(world: &mut World, place_id: String) {
    wait_player_message(world, &format!("a view at {place_id}"), |message| {
        message["kind"] == "view"
            && message["viewer"] == "rowan"
            && message["place"]["place"] == place_id
    })
    .await;
}

#[then(expr = "the player hears the GM say {string}")]
async fn the_player_hears_the_gm(world: &mut World, text: String) {
    wait_player_message(world, "the GM's say", |message| {
        message["kind"] == "say" && message["from"] == "gm" && message["text"] == text
    })
    .await;
}

#[then(expr = "the player is shown {string} with an exit to {string}")]
async fn the_player_is_shown(world: &mut World, place_name: String, place_id: String) {
    wait_player_message(world, &format!("a view of {place_name}"), |message| {
        let exits = message["place"]["exits"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        message["kind"] == "view"
            && message["place"]["name"] == place_name
            && exits.iter().any(|exit| exit["place"] == place_id)
    })
    .await;
}

#[then(expr = "the table refuses the player with {word}")]
async fn the_player_is_refused(world: &mut World, code: String) {
    wait_player_message(world, &format!("a {code} refusal"), |message| {
        message["kind"] == "refused" && message["code"] == code
    })
    .await;
}

#[then(expr = "the GM sees rowan's travel queued")]
async fn the_gm_sees_the_travel_queued(world: &mut World) {
    wait_gm_event(world, "rowan's travel queued", |event| {
        event["kind"] == "queued" && event["actor"] == "rowan" && event["verb"] == "travel"
    })
    .await;
}

#[then(expr = "the GM hears {word} say {string}")]
async fn the_gm_hears(world: &mut World, actor_id: String, text: String) {
    let from = format!("player:{actor_id}");
    wait_gm_event(world, &format!("{from}'s say"), |event| {
        event["kind"] == "say" && event["from"] == from && event["text"] == text
    })
    .await;
}

#[then(expr = "the GM sees rowan in {word}")]
async fn the_gm_sees_rowan_in(world: &mut World, place_id: String) {
    let view = host::call_gm_tool(world, "perceive", r#"{"as":"rowan"}"#).await;
    assert_eq!(view["place"]["place"], place_id, "{view}");
}

#[then(expr = "the GM sees nothing queued")]
async fn the_gm_sees_nothing_queued(world: &mut World) {
    let status = host::call_gm_tool(world, "status", "{}").await;
    assert_eq!(status["queued"], serde_json::json!([]), "{status}");
}

#[then(expr = "the GM sees no player seated")]
async fn the_gm_sees_no_player_seated(world: &mut World) {
    host::wait_players(world, 0).await;
}

/// Waits for a message body after `world.mark` that passes `done`.
async fn wait_player_message(world: &World, want: &str, done: impl Fn(&Value) -> bool) {
    host::poll_until(want, async || {
        let text = host::player_text(world);
        let after = text.get(world.mark..).unwrap_or_default();
        if parse_lines(after)
            .iter()
            .any(|message| done(&message["body"]))
        {
            Ok(())
        } else {
            Err(after.to_string())
        }
    })
    .await;
}

/// Waits for a GM event whose body passes `done`.
async fn wait_gm_event(world: &mut World, want: &str, done: impl Fn(&Value) -> bool) {
    host::wait_gm_until(world, want, |text| {
        parse_lines(text)
            .iter()
            .any(|line| done(&line["params"]["body"]))
    })
    .await;
}

/// Each complete JSON line; a line still arriving is skipped.
fn parse_lines(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[then(expr = "the player is offered {string}")]
async fn the_player_is_offered(world: &mut World, command_id: String) {
    wait_player_message(world, &format!("help with {command_id}"), |message| {
        help_command(message, &command_id).is_some()
    })
    .await;
}

#[then(expr = "the player's map lists {string}")]
async fn the_players_map_lists(world: &mut World, place_name: String) {
    wait_player_message(world, &format!("a map with {place_name}"), |message| {
        let places = message["places"].as_array().cloned().unwrap_or_default();
        message["kind"] == "map" && places.iter().any(|place| place["name"] == place_name)
    })
    .await;
}

/// The command `command_id` of a `help` body.
fn help_command<'a>(message: &'a Value, command_id: &str) -> Option<&'a Value> {
    if message["kind"] != "help" {
        return None;
    }
    let commands = message["commands"].as_array()?;
    commands.iter().find(|command| command["id"] == command_id)
}
