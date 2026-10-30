//! A move refused while its round runs tells that player why.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::journal::list_round_keys;

use super::common::{capture_play_stdout, edit_save, wait_play, wait_play_until, write_play};
use super::table;
use crate::support::{self, text_of};
use crate::World;

#[then(expr = "play's error shows {string} once")]
fn play_error_shows_once(world: &mut World, text: String) {
    let start = Instant::now();
    loop {
        let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
        let stderr = text_of(&play.stderr);
        let count = stderr.matches(&text).count();
        if count > 1 {
            panic!("saw {text} {count} times\n{stderr}");
        }
        if count == 1 {
            break;
        }
        if start.elapsed() > Duration::from_secs(8) {
            panic!("missing {text}\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // The world loop answers this look after every frame it sent before, and play prints in order.
    let mark = capture_play_stdout(world).len();
    write_play(world, "look");
    wait_play_until(world, |stdout| stdout[mark..].contains("Exits:"), "look");
    let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
    let stderr = text_of(&play.stderr);
    let count = stderr.matches(&text).count();
    assert_eq!(count, 1, "saw {text} {count} times\n{stderr}");
}

#[given(expr = "Oden's habit takes a tonic that is not there")]
fn odens_habit_takes_a_missing_tonic(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "oden-habit");
    let path = pack.join("cast/oden.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read oden: {err}"));
    let next = replace_oden_budget_and_rules(
        &text,
        "\
idle:
  eligible: true
rules:
  - id: grab
    when: habit
    do:
      - item:
          who: here
          id: tonic
          add: -1
",
    );
    fs::write(&path, next).unwrap_or_else(|err| panic!("write oden: {err}"));
    edit_save(&pack, |_| {});
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_lark");
    world.pack = Some(pack);
}

#[when(expr = "the player waits for the resume")]
fn the_player_waits_for_the_resume(world: &mut World) {
    let (mark, added) = table::player_send(world, "wait", "\"kind\":\"resume\"");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the player sees no refused frame")]
fn the_player_sees_no_refused_frame(world: &mut World) {
    assert!(
        !world.player_added.contains("\"kind\":\"refused\""),
        "{}",
        world.player_added
    );
}

#[given(expr = "a sample table where push and shove each cost 1 grit")]
fn push_and_shove_cost_grit(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "grit-gap");
    add_grit_meter(&pack);
    crate::support::append(
        &pack.join("rules/verbs.yaml"),
        "\
\n  - id: push
    when:
      verb: push
    cost:
      meters:
        grit: 1
    do: []

  - id: shove
    when:
      verb: shove
    cost:
      meters:
        grit: 1
    do: []
",
    );
    edit_save(&pack, |_| {});
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    world.pack = Some(pack);
}

#[given(expr = "a sample table where dropping the tonic locks the loft")]
fn dropping_the_tonic_locks_the_loft(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "locked-loft");
    let path = pack.join("world/places/gate_yard.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read yard: {err}"));
    let next = text.replacen(
        "  - to: loft\n    name: loft\n",
        "\
\n  - to: loft
    name: loft
    if:
      - has_item:
          who: self
          id: tonic
    locked_text: The loft door is locked.
",
        1,
    );
    assert_ne!(text, next, "the loft exit was not locked");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write yard: {err}"));
    edit_save(&pack, |_| {});
    world.pack = Some(pack);
}

#[given(expr = "a sample table with the GM seated")]
fn a_sample_table_with_the_gm_seated(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "named-gap");
    edit_save(&pack, |_| {});
    seat_gm_and_lark(world, pack);
}

#[given(expr = "a sample table where one effect is over budget and the GM is seated")]
fn one_effect_is_over_budget(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "over-budget");
    let path = pack.join("rules/budget.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read budget: {err}"));
    let next = text.replacen(
        "max_effects_per_intent: 4\n",
        "max_effects_per_intent: 0\n",
        1,
    );
    assert_ne!(text, next, "the effect budget was not lowered");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write budget: {err}"));
    crate::support::append(
        &pack.join("rules/verbs.yaml"),
        "\
\n  - id: tap
    when:
      verb: tap
    do:
      - flag:
          who: self
          key: tapped
          value: true
",
    );
    edit_save(&pack, |_| {});
    seat_gm_and_lark(world, pack);
}

#[when(expr = "the GM is seated and play joins")]
fn the_gm_is_seated_and_play_joins(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    table::seat_gm(world);
    let addr = table::game_addr(world);
    world.play = Some(support::spawn(&[
        "play",
        "--addr",
        &addr,
        "--as",
        "player:pc_lark",
    ]));
    wait_play(world, "You are in");
}

#[when(expr = "play queues {string}")]
fn play_queues(world: &mut World, line: String) {
    let before = capture_play_stdout(world)
        .matches("Your action is queued.")
        .count();
    write_play(world, &line);
    wait_play_until(
        world,
        |stdout| stdout.matches("Your action is queued.").count() > before,
        "Your action is queued.",
    );
}

#[when(expr = "Lark queues {string}")]
fn lark_queues(world: &mut World, line: String) {
    let (mark, added) = table::player_send(world, &line, "\"kind\":\"queued\"");
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the GM sends Lark's strike on {word}")]
fn the_gm_sends_larks_strike(world: &mut World, target: String) {
    let args = format!(r#"{{"actor_id":"pc_lark","move":"strike","target":"{target}"}}"#);
    let text = table::gm_call(world, 4, "queue_intent", &args);
    assert!(text.contains("\"id\""), "{text}");
}

#[then(expr = "Lark's refused frame for {string} has no missing field")]
fn larks_refused_frame_has_no_missing_field(world: &mut World, code: String) {
    let line = format!("{{\"kind\":\"refused\",\"code\":\"{code}\"}}");
    let added = table::wait_player_from(world, world.mark, &line);
    let frames = added
        .lines()
        .filter(|row| row.contains("\"kind\":\"refused\""))
        .collect::<Vec<_>>();
    assert_eq!(
        frames.iter().filter(|row| **row == line).count(),
        1,
        "{added}"
    );
    assert!(
        frames.iter().all(|row| !row.contains("\"detail\"")),
        "{added}"
    );
}

#[given(expr = "Oden can pay for a habit until Lark spends his grit")]
fn oden_can_pay_until_lark_spends_his_grit(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "oden-grit");
    add_grit_meter(&pack);
    let path = pack.join("cast/oden.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read oden: {err}"));
    let next = replace_oden_budget_and_rules(
        &text,
        "\
idle:
  eligible: true
rules:
  - id: heave
    when: habit
    cost:
      meters:
        grit: 1
    do:
      - flag:
          who: self
          key: heaved
          value: true
",
    );
    fs::write(&path, next).unwrap_or_else(|err| panic!("write oden: {err}"));
    crate::support::append(
        &pack.join("rules/verbs.yaml"),
        "\
\n  - id: sap
    when:
      verb: sap
    do:
      - meter:
          who: npc_oden
          key: grit
          add: -1
",
    );
    edit_save(&pack, |_| {});
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_lark");
    world.pack = Some(pack);
}

#[when(expr = "Lark saps Oden")]
fn lark_saps_oden(world: &mut World) {
    let (mark, added) = table::player_send(world, "sap", "\"kind\":\"resume\"");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the round rejects that habit with {string}")]
fn the_round_rejects_that_habit_with(world: &mut World, word: String) {
    assert_one_rejection(world, &word);
}

fn seat_gm_and_lark(world: &mut World, pack: std::path::PathBuf) {
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "pc_lark");
    world.pack = Some(pack);
}

fn replace_oden_budget_and_rules(text: &str, rules: &str) -> String {
    let start = text.find("idle:\n").expect("oden round");
    let end = text.find("\n---\n").expect("oden close");
    assert!(start < end, "oden round is outside the frontmatter");
    let next = format!("{}{}{}", &text[..start], rules, &text[end + 1..]);
    assert_ne!(text, next, "oden's habit was not inserted");
    next
}

fn add_grit_meter(pack: &Path) {
    crate::support::append(
        &pack.join("rules/meters.yaml"),
        "\
\n  - id: grit
    min: 0
    base_cap: 5
    max: 5
    base: 1
",
    );
}

/// `word` is a code, optionally followed by the missing meter or item.
fn assert_one_rejection(world: &World, word: &str) {
    let (code, missing) = word
        .split_once(' ')
        .map_or((word, None), |(code, missing)| (code, Some(missing)));
    let pack = table::pack(world);
    let keys = list_round_keys(pack);
    let mut hits = 0;
    for key in &keys {
        let line = spookery_engine::testing::journal::read_record_line(pack, key)
            .unwrap_or_else(|err| panic!("{}: {}", key, err.detail));
        let round: Value = serde_json::from_str(&line).unwrap_or_else(|err| panic!("{key}: {err}"));
        let Some(rejected) = round["refused"].as_array() else {
            continue;
        };
        hits += rejected
            .iter()
            .filter(|item| item["code"] == code && item["detail"].as_str() == missing)
            .count();
    }
    assert_eq!(hits, 1, "{keys:?}");
}
