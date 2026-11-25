//! `crates/protocol/sessions/v4.jsonl`, a session recorded against wire version 4, replayed
//! against this build. Each line is one message a seat sent or got. A seat's id is
//! its endpoint, `gm` or `player:<actor>`, and it connects at its first line. The seats send
//! theirs in order, each after the reply to the one before, and every message a seat got
//! must still reach it.

use std::path::PathBuf;

use cucumber::{given, then, when};
use serde_json::{json, Value};
use tungstenite::Message;

use crate::support::{self, copy_pack};
use crate::World;

use super::host::{self, Seat};

#[given(expr = "a session recorded against this wire version")]
fn a_recorded_session(world: &mut World) {
    // repeats module: mirrors its step text
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../protocol/sessions/v4.jsonl");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    world.recorded = parse_lines(&text);
}

/// Replays the sends and writes this build's own recording of the session to a fresh dir.
#[when(expr = "CI replays it against this build")]
async fn ci_replays_it(world: &mut World) {
    let pack = copy_pack("docks", "recorded-session");
    host::listen(world, &pack).await;
    world.pack = Some(pack);
    let port = host::live_mut(world).game_port;
    let mut seats: Vec<(String, Seat)> = Vec::new();
    let mut read = Vec::new();
    let mut own_recording = String::new();
    for line in &world.recorded {
        let seat_id = line["seat"]
            .as_str()
            .unwrap_or_else(|| panic!("no seat\n{line}"));
        if !seats.iter().any(|(seat, _)| seat == seat_id) {
            let endpoint = seat_id.split(':').next().unwrap_or(seat_id);
            let seat = Seat::open(port, endpoint).await;
            host::wait_suffix(&seat.text, 0, "\n").await;
            seats.push((seat_id.to_string(), seat));
            read.push(0);
        }
        let Some(message) = line.get("send") else {
            continue;
        };
        let at = seats.iter().position(|(seat, _)| seat == seat_id).unwrap();
        let id = message["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a send has no id\n{line}"));
        seats[at].1.send_raw(Message::text(message.to_string()));
        own_recording.push_str(&format!("{}\n", json!({"seat": seat_id, "send": message})));
        seats[at].1.wait_reply(id).await;
        for ((seat_id, seat), mark) in seats.iter().zip(&mut read) {
            let text = support::text_of(&seat.text);
            for got in parse_lines(&text[*mark..]) {
                let got = scrub_tokens(got);
                own_recording.push_str(&format!("{}\n", json!({"seat": seat_id, "get": got})));
            }
            *mark = text.len();
        }
    }
    world.own_recording_path =
        support::fresh_dir("recorded-session").join("spook-recorded-session.jsonl");
    std::fs::write(&world.own_recording_path, own_recording).unwrap_or_else(|err| panic!("{err}"));
    world.replayed = seats;
}

#[then(expr = "every server message the old client knows is unchanged")]
async fn every_known_message_is_unchanged(world: &mut World) {
    for (seat_id, seat) in &world.replayed {
        let want: Vec<Value> = world
            .recorded
            .iter()
            .filter(|line| line["seat"] == seat_id.as_str())
            .filter_map(|line| line.get("get").cloned())
            .collect();
        assert!(
            !want.is_empty(),
            "the recording has no message {seat_id} got"
        );
        host::poll_until(&format!("every message {seat_id} got"), async || {
            let got = parse_lines(&support::text_of(&seat.text));
            match first_lost(&want, &got) {
                None => Ok(()),
                Some(lost) => Err(format!(
                    "{seat_id} no longer gets {lost}\nthis build's session: {}",
                    world.own_recording_path.display()
                )),
            }
        })
        .await;
    }
}

/// The first of `old` that `live` no longer says, in order.
fn first_lost<'a>(old: &'a [Value], live: &[Value]) -> Option<&'a Value> {
    let mut live = live.iter();
    old.iter()
        .find(|item| !live.any(|candidate| still_says(item, candidate)))
}

/// Whether `live` still says all `old` said: every field with the same value, and every
/// item of a list in order among its own. `seq` and `token` need only be there, since a new
/// kind moves every later `seq` and each host mints its own tokens.
fn still_says(old: &Value, live: &Value) -> bool {
    match (old, live) {
        (Value::Object(old), Value::Object(live)) => {
            old.iter().all(|(key, value)| match live.get(key) {
                Some(_) if key == "seq" || key == "token" => true,
                Some(live) => still_says(value, live),
                None => false,
            })
        }
        (Value::Array(old), Value::Array(live)) => first_lost(old, live).is_none(),
        _ => old == live,
    }
}

/// `body.token` as `<token>`, so a recording does not change with each host's tokens.
fn scrub_tokens(mut message: Value) -> Value {
    if message["body"].get("token").is_some() {
        message["body"]["token"] = json!("<token>");
    }
    message
}

fn parse_lines(text: &str) -> Vec<Value> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}: {line}")))
        .collect()
}
