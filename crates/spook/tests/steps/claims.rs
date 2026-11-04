//! A claim's token: a dropped player takes its PC back and catches up on what it missed.

use std::io::Write;

use cucumber::{given, then, when};
use serde_json::{json, Value};

use crate::World;

use super::common;
use super::table::{self, PlayerClient, Seat};

/// Opens a player connection and claims Rowan. `scratch` keeps the token and `mark` the
/// `seq` of the view that ends the welcome.
#[given(expr = "a player claimed rowan and got token t1")]
fn a_player_claimed_rowan(world: &mut World) {
    if world.live.is_none() {
        let pack = table::pack(world).to_path_buf();
        table::listen(world, &pack);
    }
    let seat = Seat::open(table::live_mut(world).game_port, "player");
    let id = seat.send("claim", json!({ "actor": "pc_rowan" }));
    let reply = parse(&seat.wait_reply(&id));
    assert_eq!(reply["body"]["kind"], "claimed", "{reply}");
    let view = table::wait_suffix(&seat.text, 0, "\"kind\":\"view\"");
    let view = parse(view.lines().last().unwrap_or_default());
    world.scratch = reply["body"]["token"]
        .as_str()
        .unwrap_or_else(|| panic!("no token\n{reply}"))
        .to_string();
    world.mark = seq_of(&view) as usize;
    table::live_mut(world).players.push(PlayerClient {
        actor_id: "pc_rowan".to_string(),
        seat,
    });
}

#[when(expr = "the connection drops without release")]
fn the_connection_drops(world: &mut World) {
    table::live_mut(world)
        .players
        .retain(|player| player.actor_id != "pc_rowan");
}

#[when(expr = "the connection drops without release and {int} server messages pass")]
fn the_connection_drops_and_messages_pass(world: &mut World, count: usize) {
    the_connection_drops(world);
    if table::live_mut(world).gm.is_none() {
        table::seat_gm(world);
    }
    for index in 1..=count {
        let args = json!({ "address": "table", "text": format!("line {index}") });
        let said = table::call_gm_tool(world, "say", &args.to_string());
        assert!(said.get("code").is_none(), "{said}");
    }
}

#[when(expr = "the player releases rowan")]
fn the_player_releases_rowan(world: &mut World) {
    table::close_actor(world, "pc_rowan");
    the_connection_drops(world);
}

/// A new connection claims by token, after the last `seq` the old one read. The reply
/// goes to `player_added`.
#[when(expr = "the player reconnects with token t1")]
fn the_player_reconnects(world: &mut World) {
    let seat = Seat::open(table::live_mut(world).game_port, "player");
    let args = json!({ "token": world.scratch, "last_seq": world.mark });
    let id = seat.send("claim", args);
    world.player_added = seat.wait_reply(&id);
    table::live_mut(world).players.push(PlayerClient {
        actor_id: "pc_rowan".to_string(),
        seat,
    });
}

#[then(expr = "it holds rowan again and receives the messages after its last seq")]
fn it_holds_rowan_and_catches_up(world: &mut World) {
    let missed = held_again(world, false);
    let texts: Vec<String> = (1..=100).map(|index| format!("line {index}")).collect();
    assert_eq!(says_in(&missed), texts);
    let player = table::player_client(world, "pc_rowan");
    let id = player.seat.send("look", json!({}));
    let looked = parse(&player.seat.wait_reply(&id));
    assert_eq!(looked["body"]["kind"], "lines", "{looked}");
}

#[then(expr = "it holds rowan again with a gap and the newest 256 messages")]
fn it_holds_rowan_with_a_gap(world: &mut World) {
    let missed = held_again(world, true);
    let texts: Vec<String> = (45..=300).map(|index| format!("line {index}")).collect();
    assert_eq!(says_in(&missed), texts);
}

#[then(expr = "a GM intent for rowan is refused while it is held")]
fn a_gm_intent_for_rowan_is_refused(world: &mut World) {
    assert_eq!(queue_rowan_wait(world)["code"], "ACTOR_MISMATCH");
}

#[then(expr = "a GM intent for rowan is accepted")]
fn a_gm_intent_for_rowan_is_accepted(world: &mut World) {
    let queued = queue_rowan_wait(world);
    assert_eq!(queued["kind"], "queued", "{queued}");
}

#[then(expr = "another player's claim on rowan is refused with {string}")]
fn another_claim_is_refused(world: &mut World, code: String) {
    let reply = parse(&table::claim(world, "pc_rowan"));
    assert_eq!(reply["body"]["code"], code.as_str(), "{reply}");
}

#[then(expr = "the reconnect is refused with {string}")]
fn the_reconnect_is_refused(world: &mut World, code: String) {
    let reply = parse(&world.player_added);
    assert_eq!(reply["body"]["code"], code.as_str(), "{reply}");
}

/// The reconnect's `claimed` reply holds Rowan under the same token, with `gap` as given.
/// Returns its `missed` messages, each after the old connection's last `seq`.
fn held_again(world: &World, gap: bool) -> Vec<Value> {
    let reply = parse(&world.player_added);
    let body = &reply["body"];
    assert_eq!(body["kind"], "claimed", "{reply}");
    assert_eq!(body["actor"], "pc_rowan", "{reply}");
    assert_eq!(body["token"], world.scratch.as_str(), "{reply}");
    assert_eq!(body["gap"].as_bool().unwrap_or(false), gap, "{reply}");
    let missed = body["missed"].as_array().cloned().unwrap_or_default();
    let seqs: Vec<u64> = missed.iter().map(seq_of).collect();
    assert!(seqs.is_sorted(), "{seqs:?}");
    assert!(seqs.iter().all(|&seq| seq > world.mark as u64), "{seqs:?}");
    assert!(seqs.iter().all(|&seq| seq < seq_of(&reply)), "{seqs:?}");
    missed
}

fn says_in(missed: &[Value]) -> Vec<String> {
    missed
        .iter()
        .map(|message| {
            assert_eq!(message["body"]["kind"], "say", "{message}");
            message["body"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

fn queue_rowan_wait(world: &mut World) -> Value {
    if table::live_mut(world).gm.is_none() {
        table::seat_gm(world);
    }
    table::call_gm_tool(world, "queue", r#"{"actor_id":"pc_rowan","move":"wait"}"#)
}

fn parse(line: &str) -> Value {
    serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}\n{line}"))
}

fn seq_of(message: &Value) -> u64 {
    message["seq"]
        .as_u64()
        .unwrap_or_else(|| panic!("no seq\n{message}"))
}

#[when(expr = "play's input ends")]
fn plays_input_ends(world: &mut World) {
    let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
    drop(play.stdin.take());
}

#[then(expr = "play exits 0")]
fn play_exits_0(world: &mut World) {
    let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
    let status = play
        .child
        .wait()
        .unwrap_or_else(|err| panic!("wait play: {err}"));
    assert!(status.success(), "{status:?}");
}

#[then(expr = "a new player can claim rowan")]
fn a_new_player_can_claim_rowan(world: &mut World) {
    let reply = parse(&table::claim(world, "pc_rowan"));
    assert_eq!(reply["body"]["kind"], "claimed", "{reply}");
}

/// A say past the 64 KiB message cap, so the table drops play's connection.
#[when(expr = "play says a line over the message cap")]
fn play_says_a_line_over_the_cap(world: &mut World) {
    let line = format!("say {}", "n".repeat(70 * 1024));
    let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
    let stdin = play.stdin.as_mut().unwrap_or_else(|| panic!("play stdin"));
    writeln!(stdin, "{line}").unwrap_or_else(|err| panic!("write play: {err}"));
    stdin
        .flush()
        .unwrap_or_else(|err| panic!("flush play: {err}"));
}

#[then(expr = "play shows the shop again as rowan")]
fn play_shows_the_shop_again(world: &mut World) {
    let shown = |stdout: &str| stdout.matches("You are in").count() >= 2;
    common::wait_play_until(world, shown, "a second view");
}
