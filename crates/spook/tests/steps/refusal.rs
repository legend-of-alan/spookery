//! A move refused while its round runs tells that player why.

use std::fs;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::journal::list_round_keys;

use super::common::{
    capture_play_stdout, edit_save, spawn_play, split_line, wait_play, wait_play_until, write_play,
};
use super::table;
use crate::support::{self, text_of};
use crate::World;

#[then(expr = "play's error shows {string} once")]
fn play_error_shows_once(world: &mut World, text: String) {
    let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
    table::poll_until(&text, || {
        let stderr = text_of(&play.stderr);
        match stderr.matches(&text).count() {
            0 => Err(stderr),
            1 => Ok(()),
            count => panic!("saw {text} {count} times\n{stderr}"),
        }
    });
    // The world loop answers this look after every frame it sent before, and play prints in order.
    let mark = capture_play_stdout(world).len();
    write_play(world, "look");
    wait_play_until(world, |stdout| stdout[mark..].contains("Exits:"), "look");
    let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
    let stderr = text_of(&play.stderr);
    let count = stderr.matches(&text).count();
    assert_eq!(count, 1, "saw {text} {count} times\n{stderr}");
}

#[given(expr = "Bram's habit takes a tonic that is not there")]
fn brams_habit_takes_a_missing_tonic(world: &mut World) {
    let pack = support::copy_pack("docks", "bram-habit");
    let path = pack.join("cast/bram.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read bram: {err}"));
    let next = replace_idle_and_rules(
        &text,
        "\
idle:
  eligible: true
rules:
  - id: grab
    when: habit
    do:
      - item:
          of: here
          id: tonic
          add: -1
",
    );
    fs::write(&path, next).unwrap_or_else(|err| panic!("write bram: {err}"));
    edit_save(&pack, |_| {});
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "harbormouse");
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

#[given(expr = "a docks table where push and shove each cost 1 vigor")]
fn push_and_shove_cost_vigor(world: &mut World) {
    let pack = support::copy_pack("docks", "vigor-gap");
    crate::support::append(
        &pack.join("rules/verbs.yaml"),
        "\
\n  - id: push
    when:
      verb: push
    cost:
      meters:
        vigor: 1
    do: []

  - id: shove
    when:
      verb: shove
    cost:
      meters:
        vigor: 1
    do: []
",
    );
    edit_save(&pack, |_| {});
    world.pack = Some(pack);
}

/// The loft is a door the tonic opens, and it stands open. Dropping an item in the yard bars
/// it again.
#[given(expr = "a docks table where dropping the tonic locks the loft")]
fn dropping_the_tonic_locks_the_loft(world: &mut World) {
    let pack = support::copy_pack("docks", "locked-loft");
    let path = pack.join("world/places/gate_yard.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read yard: {err}"));
    let next = text.replacen(
        "  - to: loft\n    id: loft\n",
        "\
\n  - to: loft
    id: loft
    if:
      - has_item:
          subject: self
          id: tonic
    locked_text: The loft door is locked.
",
        1,
    );
    let drop = "rules:
  - id: drop
    when: { verb: drop, target: item }
    do:
      - item: { of: self, id: target, add: -1 }
      - item: { of: here, id: target, add: 1 }
      - flag: { of: \"exit:gate_yard.loft\", key: open, value: false }
";
    let next = next.replacen("rules:\n", drop, 1);
    assert_ne!(text, next, "the loft exit was not locked");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write yard: {err}"));
    edit_save(&pack, |now| {
        let loft = spookery_engine::ExitId {
            place: "gate_yard".into(),
            exit: "loft".to_string(),
        };
        now.places_mut().set_exit_open(&loft, true).unwrap();
    });
    world.pack = Some(pack);
}

#[given(expr = "a docks table with a stub GM and Harbormouse seated")]
fn a_docks_table_with_a_stub_gm_and_harbormouse_seated(world: &mut World) {
    let pack = support::copy_pack("docks", "named-gap");
    seat_gm_and_harbormouse(world, pack);
}

#[given(
    expr = "a docks table with a stub GM, Harbormouse seated and a strike success_at of {string}"
)]
fn a_docks_table_with_a_strike_success_at_of(world: &mut World, success_at: String) {
    let pack = support::copy_pack("docks", "rolling-gap");
    let path = pack.join("rules/verbs.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read verbs: {err}"));
    let from = "      success_at: 38\n";
    let to = format!("      success_at: \"{success_at}\"\n");
    assert!(text.contains(from), "{text}");
    fs::write(&path, text.replacen(from, &to, 1))
        .unwrap_or_else(|err| panic!("write verbs: {err}"));
    support::stamp_pack_rev(&pack);
    seat_gm_and_harbormouse(world, pack);
}

#[then(expr = "the round records no dice line")]
fn the_round_records_no_dice_line(world: &mut World) {
    let round = super::common::read_first_round(world);
    assert!(round.get("dice").is_none(), "{round}");
}

#[given(expr = "a docks table where one effect is over budget and the GM is seated")]
fn one_effect_is_over_budget(world: &mut World) {
    let pack = support::copy_pack("docks", "over-budget");
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
          of: self
          key: tapped
          value: true
",
    );
    edit_save(&pack, |_| {});
    seat_gm_and_harbormouse(world, pack);
}

#[when(expr = "the GM is seated and play joins")]
fn the_gm_is_seated_and_play_joins(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    table::seat_gm(world);
    let addr = table::game_addr(world);
    world.play = Some(spawn_play(&addr, "harbormouse", &[]));
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

#[when(expr = "Harbormouse queues {string}")]
fn harbormouse_queues(world: &mut World, line: String) {
    let (mark, added) = table::player_send(world, &line, "\"kind\":\"queued\"");
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the GM sends Harbormouse's strike on {word}")]
fn the_gm_sends_harbormouses_strike(world: &mut World, target: String) {
    let args = format!(r#"{{"actor_id":"harbormouse","move":"strike","target":"{target}"}}"#);
    world.scratch = table::gm_tool_call(world, 4, "queue", &args);
}

#[then(expr = "the GM's queue is refused {string}")]
fn the_gms_queue_is_refused(world: &mut World, code: String) {
    let text = &world.scratch;
    assert!(text.contains(&format!("\"code\":\"{code}\"")), "{text}");
    assert!(!text.contains("\"kind\":\"queued\""), "{text}");
}

#[then(expr = "Harbormouse's refused frame for {string} has no missing field")]
fn harbormouses_refused_frame_has_no_missing_field(world: &mut World, code: String) {
    let body = serde_json::json!({ "kind": "refused", "code": code });
    let added = table::wait_player_from(world, world.mark, &format!("\"code\":\"{code}\""));
    let refusals: Vec<serde_json::Value> = added
        .lines()
        .filter_map(|row| serde_json::from_str::<serde_json::Value>(row).ok())
        .map(|message| message["body"].clone())
        .filter(|body| body["kind"] == "refused")
        .collect();
    assert_eq!(
        refusals.iter().filter(|refused| **refused == body).count(),
        1,
        "{added}"
    );
    assert!(
        refusals
            .iter()
            .all(|refused| refused.get("detail").is_none()),
        "{added}"
    );
}

#[given(expr = "Bram can pay for a habit until Harbormouse spends Bram's vigor")]
fn bram_can_pay_until_harbormouse_spends_brams_vigor(world: &mut World) {
    let pack = support::copy_pack("docks", "bram-vigor");
    let path = pack.join("cast/bram.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read bram: {err}"));
    let next = replace_idle_and_rules(
        &text,
        "\
idle:
  eligible: true
flags:
  - heaved
rules:
  - id: heave
    when: habit
    cost:
      meters:
        vigor: 1
    do:
      - flag:
          of: self
          key: heaved
          value: true
",
    );
    fs::write(&path, next).unwrap_or_else(|err| panic!("write bram: {err}"));
    crate::support::append(
        &pack.join("rules/verbs.yaml"),
        "\
\n  - id: sap
    when:
      verb: sap
    do:
      - meter:
          of: bram
          key: vigor
          add: -1
",
    );
    edit_save(&pack, |_| {});
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "harbormouse");
    world.pack = Some(pack);
}

#[when(expr = "Harbormouse saps Bram")]
fn harbormouse_saps_bram(world: &mut World) {
    let (mark, added) = table::player_send(world, "sap", "\"kind\":\"resume\"");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "play does not show the round's intent ids")]
fn play_does_not_show_the_intent_ids(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let stdout = capture_play_stdout(world);
    let mut ids = Vec::new();
    for key in list_round_keys(&pack) {
        let line = spookery_engine::testing::journal::read_record_line(&pack, &key)
            .unwrap_or_else(|err| panic!("{key}: {}", err.detail));
        let round: Value = serde_json::from_str(&line).unwrap_or_else(|err| panic!("{key}: {err}"));
        let intents = round["intents"].as_array().cloned().unwrap_or_default();
        ids.extend(
            intents
                .iter()
                .filter_map(|intent| Some(intent["id"].as_str()?.to_string())),
        );
    }
    assert!(!ids.is_empty(), "no intents");
    for id in ids {
        assert!(!stdout.contains(&id), "{id}\n{stdout}");
    }
}

#[then(expr = "the round rejects that habit with {string}")]
fn the_round_rejects_that_habit_with(world: &mut World, word: String) {
    assert_one_rejection(world, &word);
}

fn seat_gm_and_harbormouse(world: &mut World, pack: std::path::PathBuf) {
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "harbormouse");
    world.pack = Some(pack);
}

/// Replace a cast file's `idle` block and the rest of its frontmatter with `block`.
fn replace_idle_and_rules(text: &str, block: &str) -> String {
    let start = text.find("idle:\n").expect("the idle block");
    let end = text.find("\n---\n").expect("the frontmatter end");
    assert!(start < end, "the idle block is outside the frontmatter");
    let next = format!("{}{}{}", &text[..start], block, &text[end + 1..]);
    assert_ne!(text, next, "the habit was not inserted");
    next
}

/// `word` is a code, optionally followed by the missing meter or item.
fn assert_one_rejection(world: &World, word: &str) {
    let (code, missing) = split_line(word);
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
