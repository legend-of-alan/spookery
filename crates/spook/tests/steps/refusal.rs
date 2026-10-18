//! A move refused while its round runs tells that player why.

use std::fs;

use cucumber::{given, then, when};
use serde_json::Value;

use super::common::{edit_save, wait_play};
use super::table;
use crate::support::{self, text_of};
use crate::World;

#[given(expr = "a copy of the sample-d20 pack")]
fn a_copy_of_the_sample_d20_pack(world: &mut World) {
    world.pack = Some(support::copy_pack("sample-d20", "refused-round"));
}

#[when(expr = "play joins the sample table")]
fn play_joins_the_sample_table(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    let addr = table::game_addr(world);
    let pack_arg = support::pack_arg(&pack);
    world.play = Some(support::spawn(&["play", &pack_arg, "--addr", &addr]));
    wait_play(world, "Gate Yard");
}

#[then(expr = "play's error shows {string} once")]
fn play_error_shows_once(world: &mut World, text: String) {
    let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
    let stderr = text_of(&play.stderr);
    assert_eq!(stderr.matches(&text).count(), 1, "{stderr}");
}

#[when(expr = "the operator runs audit")]
fn the_operator_runs_audit(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["audit", &support::pack_arg(&pack)]);
}

#[given(expr = "Oden's habit takes a tonic that is not there")]
fn odens_habit_takes_a_missing_tonic(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "oden-habit");
    let path = pack.join("cast/oden.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read oden: {err}"));
    let next = text.replacen(
        "tick:\n  eligible: false\n",
        "\
tick:
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
        1,
    );
    assert_ne!(text, next, "oden's habit was not inserted");
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

#[then(expr = "the round rejects that habit with MISSING_ITEM")]
fn the_round_rejects_that_habit_with_missing_item(world: &mut World) {
    let pack = table::pack(world);
    let keys = table::list_tick_keys(pack);
    let mut hits = 0;
    for key in &keys {
        let line = spookery_kernel::api::journal::read_record_line(pack, key)
            .unwrap_or_else(|err| panic!("{}: {}", key, err.detail));
        let round: Value = serde_json::from_str(&line).unwrap_or_else(|err| panic!("{key}: {err}"));
        let Some(rejected) = round["rejected"].as_array() else {
            continue;
        };
        hits += rejected
            .iter()
            .filter(|item| item["reason"] == "MISSING_ITEM")
            .count();
    }
    assert_eq!(hits, 1, "{keys:?}");
}
