//! The game port's WebSocket envelope: the hello, numbered replies, and a GM session over it.

use cucumber::{then, when};
use serde_json::{json, Value};
use tungstenite::Message;

use crate::support;
use crate::World;

use super::host::{self, Seat};

#[when(expr = "a player connects to the game port")]
async fn a_player_connects_to_the_game_port(world: &mut World) {
    let pack = host::pack(world).to_path_buf();
    host::listen(world, &pack).await;
    let seat = Seat::open(host::live_mut(world).game_port, "player").await;
    host::wait_suffix(&seat.text, 0, "\"kind\":\"hello\"").await;
    world.scratch = support::text_of(&seat.text);
    host::live_mut(world).unclaimed = Some(seat);
}

#[then(regex = r"^the hello lists the version and the commands (.+)$")]
fn the_hello_lists_the_version_and_the_commands(world: &mut World, commands: String) {
    let hello = parse_messages(&world.scratch)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no hello\n{}", world.scratch));
    let body = &hello["body"];
    assert_eq!(body["kind"], "hello", "{hello}");
    assert_eq!(body["version"], spookery::protocol::VERSION, "{hello}");
    assert!(hello.get("seq").is_none(), "{hello}");
    let listed = body["commands"]
        .as_array()
        .unwrap_or_else(|| panic!("no commands\n{hello}"));
    for command in commands.split(", ") {
        assert!(
            listed.iter().any(|known| known == command),
            "{command}\n{hello}"
        );
    }
}

#[when(regex = r"^it sends (\{.+\})$")]
fn it_sends(world: &mut World, text: String) {
    let mark = unclaimed(world).mark();
    unclaimed(world).send_raw(Message::text(text));
    world.mark = mark;
}

#[then(regex = r"^it gets (\{.+\})$")]
async fn it_gets(world: &mut World, text: String) {
    let want: Value = serde_json::from_str(&text).unwrap_or_else(|err| panic!("{err}: {text}"));
    let seat = unclaimed(world);
    let got = host::wait_suffix(&seat.text, world.mark, "\n").await;
    let got: Value = serde_json::from_str(got.lines().next().unwrap_or_default())
        .unwrap_or_else(|err| panic!("{err}: {got}"));
    assert_eq!(got, want);
}

#[when(expr = "a GM connects on the game port")]
async fn a_gm_connects_on_the_game_port(world: &mut World) {
    let seat = Seat::open(host::live_mut(world).game_port, "gm").await;
    host::wait_suffix(&seat.text, 0, "\"kind\":\"hello\"").await;
    host::live_mut(world).ws_gm = Some(seat);
}

#[then(expr = "the WebSocket GM hears the wait queued under the player's reply seq")]
async fn the_gm_hears_the_wait_queued(world: &mut World) {
    let queued = |text: &str| {
        parse_messages(text)
            .into_iter()
            .find(|message| message["body"]["kind"] == "queued")
    };
    let player = queued(&host::player_text(world))
        .unwrap_or_else(|| panic!("no queued reply\n{}", host::player_text(world)));
    assert!(player.get("reply_to").is_some(), "{player}");
    let gm = ws_gm(world);
    let heard = host::wait_suffix(&gm.text, 0, "\"kind\":\"queued\"").await;
    let heard = queued(&heard).unwrap_or_else(|| panic!("{heard}"));
    assert_eq!(heard["body"]["actor"], "rowan", "{heard}");
    assert_eq!(heard["seq"], player["seq"], "{heard}\n{player}");
    assert!(heard.get("reply_to").is_none(), "{heard}");
}

#[when(expr = "the WebSocket GM commits")]
async fn the_websocket_gm_commits(world: &mut World) {
    world.scratch = send_gm_command(world, "commit", json!({ "include_round": true })).await;
}

#[then(expr = "the WebSocket GM gets the round and then the commit reply")]
fn the_gm_gets_the_round_then_the_reply(world: &mut World) {
    let reply: Value = serde_json::from_str(&world.scratch).unwrap();
    assert_eq!(reply["body"]["kind"], "committed", "{reply}");
    let round = parse_messages(&support::text_of(&ws_gm(world).text))
        .into_iter()
        .find(|message| message["body"]["kind"] == "round")
        .unwrap_or_else(|| panic!("no round before the reply"));
    assert!(
        round["seq"].as_u64() < reply["seq"].as_u64(),
        "{round}\n{reply}"
    );
    world.player_added = round.to_string();
}

#[then(expr = "the player hears that round under the same seq")]
async fn the_player_hears_that_round(world: &mut World) {
    let round: Value = serde_json::from_str(&world.player_added).unwrap();
    let heard = host::wait_player_from(world, world.mark, "\"kind\":\"round\"").await;
    let heard = parse_messages(&heard)
        .into_iter()
        .find(|message| message["body"]["kind"] == "round")
        .unwrap_or_else(|| panic!("{heard}"));
    assert_eq!(heard, round);
}

#[when(expr = "the WebSocket GM says {string}")]
async fn the_websocket_gm_says(world: &mut World, text: String) {
    world.scratch = send_gm_command(world, "say", json!({ "to": "table", "text": text })).await;
}

#[then(expr = "the WebSocket GM's reply is its own speech")]
fn the_gm_reply_is_its_own_speech(world: &mut World) {
    let reply: Value = serde_json::from_str(&world.scratch).unwrap();
    assert_eq!(reply["body"]["kind"], "say", "{reply}");
    assert_eq!(reply["body"]["from"], "gm", "{reply}");
    assert_eq!(reply["body"]["recipients"], json!(["rowan"]), "{reply}");
}

#[when(expr = "the WebSocket GM releases the seat")]
async fn the_websocket_gm_releases_the_seat(world: &mut World) {
    world.scratch = send_gm_command(world, "release", json!({})).await;
}

#[then(expr = "the WebSocket GM is told the seat is free")]
async fn the_gm_is_told_the_seat_is_free(world: &mut World) {
    let reply: Value = serde_json::from_str(&world.scratch).unwrap();
    assert_eq!(reply["body"]["kind"], "released", "{reply}");
    let refused = send_gm_command(world, "status", json!({})).await;
    assert!(refused.contains("\"code\":\"NOT_IN_SPEC\""), "{refused}");
}

#[then(expr = "every message the WebSocket GM got after the hello has a larger seq")]
fn every_gm_message_has_a_larger_seq(world: &mut World) {
    let text = support::text_of(&ws_gm(world).text);
    let seqs: Vec<u64> = parse_messages(&text)
        .iter()
        .skip(1)
        .map(|message| {
            message["seq"]
                .as_u64()
                .unwrap_or_else(|| panic!("no seq\n{message}"))
        })
        .collect();
    assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
}

/// Sends one GM command and returns its reply line.
async fn send_gm_command(world: &World, command: &str, args: Value) -> String {
    let gm = ws_gm(world);
    let id = gm.send(command, args);
    gm.wait_reply(&id).await
}

fn ws_gm(world: &World) -> &Seat {
    world
        .live
        .as_ref()
        .and_then(|live| live.ws_gm.as_ref())
        .unwrap_or_else(|| panic!("no GM on the game port"))
}

fn unclaimed(world: &World) -> &Seat {
    world
        .live
        .as_ref()
        .and_then(|live| live.unclaimed.as_ref())
        .unwrap_or_else(|| panic!("no WebSocket connection"))
}

fn parse_messages(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
