//! The game port's WebSocket envelope: the hello, numbered replies, and a GM session over it.

use cucumber::{then, when};
use serde_json::{json, Value};
use tungstenite::Message;

use crate::support;
use crate::World;

use super::table::{self, Seat};

#[when(expr = "a player connects over WebSocket")]
fn a_player_connects_over_websocket(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    let seat = Seat::open(table::live_mut(world).game_port, "player");
    table::wait_suffix(&seat.text, 0, "\"kind\":\"hello\"");
    world.scratch = support::text_of(&seat.text);
    table::live_mut(world).door = Some(seat);
}

#[then(regex = r"^the hello lists the version and the words (.+)$")]
fn the_hello_lists_the_version_and_the_words(world: &mut World, words: String) {
    let hello = parse_messages(&world.scratch)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no hello\n{}", world.scratch));
    let body = &hello["body"];
    assert_eq!(body["kind"], "hello", "{hello}");
    assert_eq!(body["version"], spookery::protocol::VERSION, "{hello}");
    assert!(hello.get("seq").is_none(), "{hello}");
    let listed = body["words"]
        .as_array()
        .unwrap_or_else(|| panic!("no words\n{hello}"));
    for word in words.split(", ") {
        assert!(listed.iter().any(|known| known == word), "{word}\n{hello}");
    }
}

#[when(regex = r"^it sends (\{.+\})$")]
fn it_sends(world: &mut World, text: String) {
    let mark = door(world).mark();
    door(world).send_raw(Message::text(text));
    world.mark = mark;
}

#[then(regex = r"^it gets (\{.+\})$")]
fn it_gets(world: &mut World, text: String) {
    let want: Value = serde_json::from_str(&text).unwrap_or_else(|err| panic!("{err}: {text}"));
    let seat = door(world);
    let got = table::wait_suffix(&seat.text, world.mark, "\n");
    let got: Value = serde_json::from_str(got.lines().next().unwrap_or_default())
        .unwrap_or_else(|err| panic!("{err}: {got}"));
    assert_eq!(got, want);
}

#[when(expr = "a GM sits on the game port")]
fn a_gm_sits_on_the_game_port(world: &mut World) {
    let seat = Seat::open(table::live_mut(world).game_port, "gm");
    table::wait_suffix(&seat.text, 0, "\"kind\":\"hello\"");
    table::live_mut(world).ws_gm = Some(seat);
}

#[then(expr = "the WebSocket GM hears the wait queued under the player's reply seq")]
fn the_gm_hears_the_wait_queued(world: &mut World) {
    let queued = |text: &str| {
        parse_messages(text)
            .into_iter()
            .find(|message| message["body"]["kind"] == "queued")
    };
    let player = queued(&table::player_text(world))
        .unwrap_or_else(|| panic!("no queued reply\n{}", table::player_text(world)));
    assert!(player.get("reply_to").is_some(), "{player}");
    let gm = ws_gm(world);
    let heard = table::wait_suffix(&gm.text, 0, "\"kind\":\"queued\"");
    let heard = queued(&heard).unwrap_or_else(|| panic!("{heard}"));
    assert_eq!(heard["body"]["actor"], "pc_rowan", "{heard}");
    assert_eq!(heard["seq"], player["seq"], "{heard}\n{player}");
    assert!(heard.get("reply_to").is_none(), "{heard}");
}

#[when(expr = "the WebSocket GM commits")]
fn the_websocket_gm_commits(world: &mut World) {
    world.scratch = send_gm_word(world, "commit", json!({ "include_round": true }));
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
fn the_player_hears_that_round(world: &mut World) {
    let round: Value = serde_json::from_str(&world.player_added).unwrap();
    let heard = table::wait_player_from(world, world.mark, "\"kind\":\"round\"");
    let heard = parse_messages(&heard)
        .into_iter()
        .find(|message| message["body"]["kind"] == "round")
        .unwrap_or_else(|| panic!("{heard}"));
    assert_eq!(heard, round);
}

#[when(expr = "the WebSocket GM says {string}")]
fn the_websocket_gm_says(world: &mut World, text: String) {
    world.scratch = send_gm_word(world, "say", json!({ "address": "table", "text": text }));
}

#[then(expr = "the WebSocket GM's reply is its own say")]
fn the_gm_reply_is_its_own_say(world: &mut World) {
    let reply: Value = serde_json::from_str(&world.scratch).unwrap();
    assert_eq!(reply["body"]["kind"], "say", "{reply}");
    assert_eq!(reply["body"]["from"], "gm", "{reply}");
    assert_eq!(reply["body"]["recipients"], json!(["pc_rowan"]), "{reply}");
}

#[when(expr = "the WebSocket GM releases the seat")]
fn the_websocket_gm_releases_the_seat(world: &mut World) {
    world.scratch = send_gm_word(world, "release", json!({}));
}

#[then(expr = "the WebSocket GM is told the seat is free")]
fn the_gm_is_told_the_seat_is_free(world: &mut World) {
    let reply: Value = serde_json::from_str(&world.scratch).unwrap();
    assert_eq!(reply["body"]["kind"], "released", "{reply}");
    let refused = send_gm_word(world, "status", json!({}));
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

/// Sends one GM word and returns its reply line.
fn send_gm_word(world: &World, word: &str, args: Value) -> String {
    let gm = ws_gm(world);
    let id = gm.send(word, args);
    gm.wait_reply(&id)
}

fn ws_gm(world: &World) -> &Seat {
    world
        .live
        .as_ref()
        .and_then(|live| live.ws_gm.as_ref())
        .unwrap_or_else(|| panic!("no GM on the game port"))
}

fn door(world: &World) -> &Seat {
    world
        .live
        .as_ref()
        .and_then(|live| live.door.as_ref())
        .unwrap_or_else(|| panic!("no WebSocket connection"))
}

fn parse_messages(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
