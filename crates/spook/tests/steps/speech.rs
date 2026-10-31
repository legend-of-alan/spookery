use std::fs;

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::copy_docks;
use crate::World;

use super::table;

#[when(expr = "the player says {string}")]
fn the_player_says(world: &mut World, text: String) {
    let _ = table::player_send(world, &format!("say {text}"), "\"kind\":\"say\"");
}

#[when(expr = "say to mira is submitted on an empty table saying {string}")]
fn say_to_mira_on_an_empty_table(world: &mut World, text: String) {
    let pack = table::pack(world);
    let opened = spookery_engine::testing::resume(pack);
    world.now_bytes = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    opened
        .say_as_player(
            &spookery_engine::Presence::default(),
            "pc_rowan",
            spookery_engine::Address::Actor("mira"),
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
            address: "table".to_string(),
            place: None,
            actor: None,
            recipients: vec!["pc_rowan".to_string()],
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

#[then(expr = "the tape address is {string}")]
fn the_tape_address_is(world: &mut World, address: String) {
    let tape = tape_text(world);
    assert!(
        tape.contains(&format!("\"address\":\"{address}\"")),
        "{tape}"
    );
}

#[then(expr = "the tape names the actor {string}")]
fn the_tape_names_the_actor(world: &mut World, actor: String) {
    let tape = tape_text(world);
    assert!(tape.contains(&format!("\"actor\":\"{actor}\"")), "{tape}");
}

#[then(expr = "the tape recipients are {string}")]
fn the_tape_recipients_are(world: &mut World, id: String) {
    let tape = tape_text(world);
    assert!(
        tape.contains(&format!("\"recipients\":[\"{id}\"]")),
        "{tape}"
    );
}

#[then(expr = "the tape names the place {string}")]
fn the_tape_names_the_place(world: &mut World, place: String) {
    let tape = tape_text(world);
    assert!(tape.contains(&format!("\"place\":\"{place}\"")), "{tape}");
}

fn tape_text(world: &World) -> String {
    fs::read_to_string(table::pack(world).join("state/tape.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"))
}

#[then(expr = "state_rev is unchanged")]
fn state_rev_is_unchanged(world: &mut World) {
    let rev = table::live_mut(world).seated_rev;
    assert_eq!(table::now_of(table::pack(world)).state_rev, rev);
}

#[then(expr = "save.yaml matches the bytes from before the line")]
fn save_matches_the_bytes_from_before_the_line(world: &mut World) {
    let now = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    assert_eq!(now, world.now_bytes);
}

#[given(expr = "a docks table with {string} seated")]
fn a_docks_table_with_one_seated(world: &mut World, actor: String) {
    let pack = copy_docks("speech");
    table::listen(world, &pack);
    table::seat_actor(world, &pack, &actor);
    world.pack = Some(pack);
}

#[given(expr = "a docks table with {string} and {string} seated")]
fn a_docks_table_with_two_seated(world: &mut World, first: String, second: String) {
    let pack = copy_docks("speech");
    table::listen(world, &pack);
    table::seat_actor(world, &pack, &first);
    table::seat_actor(world, &pack, &second);
    world.pack = Some(pack);
}

#[given(expr = "the GM has said {int} numbered lines to {string}")]
fn the_gm_has_said_numbered_lines(world: &mut World, count: usize, actor: String) {
    say_lines(world, &actor, count);
}

#[given(expr = "the GM has said {int} lines to {string} and then {int} lines to {string}")]
fn the_gm_has_said_lines_then_more(
    world: &mut World,
    first_count: usize,
    first: String,
    second_count: usize,
    second: String,
) {
    say_lines(world, &first, first_count);
    let lines = say_lines(world, &second, second_count);
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

#[then(expr = "the replayed say lines are {string} through {string}, in order")]
fn the_replayed_say_lines_are(world: &mut World, start: String, end: String) {
    let start = line_index(&start);
    let end = line_index(&end);
    let expected: Vec<String> = (start..=end).map(|index| format!("line{index}")).collect();
    let actor = latest_actor(world);
    assert_eq!(replay_texts(world, &actor), expected);
}

#[then(expr = "each replayed line has {string}: true")]
fn each_replayed_line_has_flag(world: &mut World, field: String) {
    assert_eq!(field, "replay");
    let actor = latest_actor(world);
    let says = replay_says(world, &actor);
    assert!(!says.is_empty(), "no replayed say lines");
    for say in &says {
        assert_eq!(say.get("replay"), Some(&Value::Bool(true)), "{say}");
    }
}

#[then(expr = "the newest replayed say line is {string}")]
fn the_newest_replayed_say_line_is(world: &mut World, text: String) {
    let actor = latest_actor(world);
    assert_eq!(replay_texts(world, &actor).last(), Some(&text));
}

#[then(expr = "{string} gets exactly those {int} lines, in order")]
fn gets_exactly_those_lines(world: &mut World, actor: String, count: usize) {
    let expected: Vec<String> = world.scratch.lines().map(str::to_string).collect();
    assert_eq!(expected.len(), count, "{}", world.scratch);
    assert_eq!(replay_texts(world, &actor), expected);
}

fn say_lines(world: &mut World, actor: &str, count: usize) -> Vec<String> {
    if table::live_mut(world).gm.is_none() {
        table::seat_gm(world);
    }
    let mut texts = Vec::with_capacity(count);
    for index in 0..count {
        let text = format!("line{index}");
        let args = format!(r#"{{"address":"actor","actor":"{actor}","text":"{text}"}}"#);
        let said = table::gm_call(world, (index as u64) + 1, "say", &args);
        let value: Value =
            serde_json::from_str(&said).unwrap_or_else(|err| panic!("say json: {err}: {said}"));
        let body = &value["result"]["structuredContent"];
        let addressed = body
            .get("recipients")
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
    replay_says(world, actor)
        .iter()
        .map(|say| {
            say.get("text")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("say text\n{say}"))
                .to_string()
        })
        .collect()
}

fn replay_says(world: &World, actor: &str) -> Vec<Value> {
    let mut says = Vec::new();
    for line in table::actor_text(world, actor).lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("kind").and_then(Value::as_str) == Some("say") {
            says.push(value);
        }
    }
    says
}

fn line_index(text: &str) -> usize {
    text.strip_prefix("line")
        .and_then(|rest| rest.parse().ok())
        .unwrap_or_else(|| panic!("{text} is not a numbered line"))
}
