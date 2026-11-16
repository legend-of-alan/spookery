use spookery_engine::testing::{place_of, state_of};
use std::fs;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery::protocol::{ServerBody, ServerMessage};
use spookery_client::text;

use crate::support::{self, copy_pack};
use crate::World;

use super::table;

#[when(expr = "the player says {string}")]
fn the_player_says(world: &mut World, text: String) {
    let _ = table::player_send(world, &format!("say {text}"), "\"kind\":\"say\"");
}

#[when(expr = "the player says {string} to the place")]
fn the_player_says_to_the_place(world: &mut World, text: String) {
    let place = place_of(&state_of(table::pack(world)), "rowan");
    let args = serde_json::json!({ "to": { "place": place }, "text": text });
    say_as_rowan(world, args);
}

#[when(expr = "the player says {string} to {word}")]
fn the_player_says_to(world: &mut World, text: String, actor: String) {
    let args = serde_json::json!({ "to": { "actor": actor }, "text": text });
    say_as_rowan(world, args);
}

fn say_as_rowan(world: &World, args: Value) {
    let seat = &table::player_client(world, "rowan").seat;
    let id = seat.send("say", args);
    let reply = seat.wait_reply(&id);
    assert!(reply.contains("\"kind\":\"say\""), "{reply}");
}

#[when(expr = "the GM says {string} to the place {string} as {string}")]
fn the_gm_says_as(world: &mut World, text: String, place: String, speaker: String) {
    if table::live_mut(world).gm.is_none() {
        table::seat_gm(world);
    }
    world.before = table::read_files(table::pack(world));
    let args = serde_json::json!({ "to": { "place": place }, "text": text, "as": speaker });
    world.scratch = table::gm_tool_call(world, 1, "say", &args.to_string());
}

#[when(expr = "{string} says {string} to the table as {string}")]
fn says_to_the_table_as(world: &mut World, actor: String, text: String, speaker: String) {
    world.before = table::read_files(table::pack(world));
    let seat = &table::player_client(world, &actor).seat;
    let args = serde_json::json!({ "to": "table", "text": text, "as": speaker });
    let id = seat.send("say", args);
    world.scratch = seat.wait_reply(&id);
}

#[then(expr = "that say is refused SCHEMA and nothing was written")]
fn that_say_is_refused(world: &mut World) {
    assert!(
        world.scratch.contains("\"code\":\"SCHEMA\""),
        "{}",
        world.scratch
    );
    assert_eq!(table::read_files(table::pack(world)), world.before);
}

#[then(expr = "{string} reads {string} under the GM's seq")]
fn reads_under_the_gms_seq(world: &mut World, actor: String, line: String) {
    let result: Value = serde_json::from_str(&world.scratch)
        .unwrap_or_else(|err| panic!("result json: {err}\n{}", world.scratch));
    let reply = result["result"]["structuredContent"]["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .unwrap_or_else(|| panic!("no reply\n{result}"));
    let speech = wait_for_speech(world, &actor, 0, &line);
    assert_eq!(speech.seq, reply["seq"].as_u64(), "{reply}");
}

/// The first speech after `mark` that `actor`'s client writes as `line`.
pub(super) fn wait_for_speech(
    world: &World,
    actor: &str,
    mark: usize,
    line: &str,
) -> ServerMessage {
    let seen = table::player_client(world, actor).text();
    table::poll_until(line, || {
        let added = support::text_of(seen)
            .get(mark..)
            .unwrap_or_default()
            .to_string();
        let speech = added
            .lines()
            .filter_map(|line| serde_json::from_str::<ServerMessage>(line).ok())
            .find(|message| {
                matches!(message.body, ServerBody::Say(_)) && text::render(&message.body) == line
            });
        speech.ok_or(added)
    })
}

#[then(expr = "the tape line is from {string}")]
fn the_tape_line_is_from(world: &mut World, from: String) {
    let tape = tape_text(world);
    assert!(tape.contains(&format!("\"from\":\"{from}\"")), "{tape}");
}

#[when(expr = "say to aldric is submitted on an empty table saying {string}")]
fn say_to_mira_on_an_empty_table(world: &mut World, text: String) {
    let pack = table::pack(world);
    let opened = spookery_engine::testing::resume(pack);
    world.now_bytes = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    opened
        .say_as_player(
            &spookery_engine::Presence::default(),
            &"rowan".into(),
            &spookery_protocol::Audience::Actor("aldric".into()),
            &text,
        )
        .unwrap_or_else(|err| panic!("speak: {err}"));
}

#[given(expr = "the tape already holds a full 1 MiB of old speech")]
fn the_tape_already_holds_a_full_mib(world: &mut World) {
    let mut lines = String::new();
    for seq in 1.. {
        let mut line = spookery_engine::emit_tape_record(&spookery_engine::TapeRecord {
            seq,
            from: spookery::protocol::Speaker::Gm,
            from_name: String::new(),
            to: spookery_protocol::Audience::Table,
            recipient_ids: vec!["rowan".to_string()],
            text: format!("old line {seq}"),
            round_index: 0,
            world_time: 0,
        });
        line.push('\n');
        if lines.len() + line.len() > 1024 * 1024 {
            break;
        }
        lines.push_str(&line);
    }
    fs::write(table::pack(world).join("state/tape.jsonl"), lines)
        .unwrap_or_else(|err| panic!("write tape: {err}"));
}

#[then(expr = "the tape contains the text {string}")]
fn the_tape_contains(world: &mut World, text: String) {
    let tape = tape_text(world);
    assert!(tape.contains(&format!("\"text\":\"{text}\"")), "{tape}");
}

#[then(expr = "the tape line is said to the table")]
fn the_tape_line_is_said_to_the_table(world: &mut World) {
    let tape = tape_text(world);
    assert!(tape.contains("\"to\":\"table\""), "{tape}");
}

#[then(expr = "the tape line is said to the {word} {string}")]
fn the_tape_line_is_said_to(world: &mut World, audience: String, id: String) {
    let tape = tape_text(world);
    let to = format!("\"to\":{{\"{audience}\":\"{id}\"}}");
    assert!(tape.contains(&to), "{tape}");
}

#[then(expr = "the tape recipients are {string}")]
fn the_tape_recipients_are(world: &mut World, id: String) {
    let tape = tape_text(world);
    assert!(
        tape.contains(&format!("\"recipient_ids\":[\"{id}\"]")),
        "{tape}"
    );
}

fn tape_text(world: &World) -> String {
    fs::read_to_string(table::pack(world).join("state/tape.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"))
}

#[then(expr = "state_rev is unchanged")]
fn state_rev_is_unchanged(world: &mut World) {
    let rev = table::live_mut(world).seated_rev;
    assert_eq!(state_of(table::pack(world)).state_rev, rev);
}

#[then(expr = "save.yaml matches the bytes from before the line")]
fn save_matches_the_bytes_from_before_the_line(world: &mut World) {
    let now = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    assert_eq!(now, world.now_bytes);
}

#[given(expr = "a docks table with {string} seated")]
fn a_docks_table_with_one_seated(world: &mut World, actor: String) {
    let pack = copy_pack("docks", "speech");
    table::listen(world, &pack);
    table::seat_actor(world, &pack, &actor);
    world.pack = Some(pack);
}

#[given(expr = "a docks table with {string} and {string} seated")]
fn a_docks_table_with_two_seated(world: &mut World, first: String, second: String) {
    let pack = copy_pack("docks", "speech");
    table::listen(world, &pack);
    table::seat_actor(world, &pack, &first);
    table::seat_actor(world, &pack, &second);
    world.pack = Some(pack);
}

#[given(expr = "the GM has said {int} numbered lines to {string}")]
fn the_gm_has_said_numbered_lines(world: &mut World, count: usize, actor: String) {
    speech_lines(world, &actor, count);
}

#[given(expr = "the GM has said {int} lines to {string} and then {int} lines to {string}")]
fn the_gm_has_said_lines_then_more(
    world: &mut World,
    first_count: usize,
    first: String,
    second_count: usize,
    second: String,
) {
    speech_lines(world, &first, first_count);
    let lines = speech_lines(world, &second, second_count);
    world.scratch = lines.join("\n");
}

#[when(expr = "{string} leaves and sits again")]
fn leaves_and_sits_again(world: &mut World, actor: String) {
    table::close_actor(world, &actor);
    table::wait_presence(world, &format!("{actor} gone"), |_, actors| {
        !actors.contains(&actor)
    });
    table::live_mut(world)
        .players
        .retain(|player| player.actor_id != actor);
    let pack = table::pack(world).to_path_buf();
    table::seat_actor(world, &pack, &actor);
}

#[then(expr = "the replayed lines are {string} through {string}, in order")]
fn the_replayed_lines_are(world: &mut World, start: String, end: String) {
    let start = line_index(&start);
    let end = line_index(&end);
    let expected: Vec<String> = (start..=end).map(|index| format!("line{index}")).collect();
    let actor = latest_actor(world);
    assert_eq!(replay_texts(world, &actor), expected);
}

#[then(expr = "the replayed lines ride in the claim's reply")]
fn the_replayed_lines_ride_in_the_claims_reply(world: &mut World) {
    let actor = latest_actor(world);
    let claimed = last_claimed(world, &actor);
    assert!(claimed.get("reply_to").is_some(), "{claimed}");
    // Live speech comes after; none was said since the claim.
    let live_speech = table::actor_text(world, &actor)
        .lines()
        .filter(|line| line.contains("\"kind\":\"say\""))
        .count();
    assert_eq!(live_speech, 0, "{}", table::actor_text(world, &actor));
}

#[then(expr = "the newest replayed line is {string}")]
fn the_newest_replayed_line_is(world: &mut World, text: String) {
    let actor = latest_actor(world);
    assert_eq!(replay_texts(world, &actor).last(), Some(&text));
}

#[then(expr = "{string} gets exactly those {int} lines, in order")]
fn gets_exactly_those_lines(world: &mut World, actor: String, count: usize) {
    let expected: Vec<String> = world.scratch.lines().map(str::to_string).collect();
    assert_eq!(expected.len(), count, "{}", world.scratch);
    assert_eq!(replay_texts(world, &actor), expected);
}

fn speech_lines(world: &mut World, actor: &str, count: usize) -> Vec<String> {
    if table::live_mut(world).gm.is_none() {
        table::seat_gm(world);
    }
    let mut texts = Vec::with_capacity(count);
    for index in 0..count {
        let text = format!("line{index}");
        let args = format!(r#"{{"to":{{"actor":"{actor}"}},"text":"{text}"}}"#);
        let body = table::call_gm_tool(world, "say", &args);
        let addressed = body
            .get("recipient_ids")
            .and_then(Value::as_array)
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(actor)));
        assert!(addressed, "say {text} was not addressed to {actor}: {body}");
        texts.push(text);
    }
    texts
}

fn latest_actor(world: &mut World) -> String {
    table::live_mut(world)
        .players
        .last()
        .unwrap_or_else(|| panic!("no seated player"))
        .actor_id
        .clone()
}

fn replay_texts(world: &World, actor: &str) -> Vec<String> {
    replay_speech(world, actor)
        .iter()
        .map(|speech| {
            speech
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("speech text\n{speech}"))
                .to_string()
        })
        .collect()
}

/// The speech the actor's last claim replayed.
fn replay_speech(world: &World, actor: &str) -> Vec<Value> {
    let claimed = last_claimed(world, actor);
    claimed["body"]["scrollback"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The actor's last `claimed` message.
fn last_claimed(world: &World, actor: &str) -> Value {
    table::actor_text(world, actor)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .rfind(|message| message["body"]["kind"] == "claimed")
        .unwrap_or_else(|| panic!("{actor} has no claimed message"))
}

fn line_index(text: &str) -> usize {
    text.strip_prefix("line")
        .and_then(|rest| rest.parse().ok())
        .unwrap_or_else(|| panic!("{text} is not a numbered line"))
}
