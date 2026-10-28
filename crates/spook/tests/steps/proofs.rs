use cucumber::when;

use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use spookery::host::idle_clock::wall_now;
use spookery::host::{call_gm_tool, commit_then_fast_forward, on_scheduled_wake, MoveOutcome};
use spookery_engine::handle_player_move;
use spookery_engine::testing::journal::list_round_keys;
use spookery_engine::World as Shard;

use crate::support::{self, now_of, open, read_round as round_value, save_now, stamp_pack_rev};
use crate::World;

fn player_waits(round: &Value) -> Vec<String> {
    round["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .map(|intent| intent["id"].as_str().unwrap().to_string())
        .collect()
}

fn queued_line_survives_a_dropped_seat() {
    let held = support::temp_pack("held");
    let mut shard = open(&held);
    let session = shard.seat_player("pc_rowan").unwrap();
    assert!(!on_scheduled_wake(&mut shard, true, false).unwrap());
    let queued = shard.queue_wait("pc_rowan").unwrap();
    assert!(!on_scheduled_wake(&mut shard, true, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.queued_intents()[0].id, queued.id);
    assert_eq!(shard.now().world_time, 1080);
    assert!(list_round_keys(&held).is_empty());
    assert!(shard.release_player_seat(&session, "pc_rowan").unwrap());
    assert_eq!(shard.now().presence.humans, 0);
    assert!(!on_scheduled_wake(&mut shard, false, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.now().round_index, 0);
    assert!(list_round_keys(&held).is_empty());
    let _ = fs::remove_dir_all(&held);
}

fn a_reseated_player_keeps_the_queued_wait() {
    let dropped = support::temp_pack("drop-queue");
    let mut shard = open(&dropped);
    let session = shard.seat_player("pc_rowan").unwrap();
    let wall = wall_now().unwrap();
    let outcome = handle_player_move(&mut shard, "pc_rowan", "wait", None, true, &wall).unwrap();
    let MoveOutcome::Queued(queued) = outcome else {
        panic!("queued wait committed");
    };
    assert!(shard.release_player_seat(&session, "pc_rowan").unwrap());
    assert_eq!(shard.now().world_time, 1080);
    assert_eq!(shard.now().round_index, 0);
    assert_eq!(shard.queued_intents().len(), 1);
    assert!(list_round_keys(&dropped).is_empty());
    shard.seat_player("pc_rowan").unwrap();
    assert_eq!(shard.now().world_time, 1080);
    assert_eq!(shard.now().round_index, 0);
    assert_eq!(shard.queued_intents()[0].id, queued.id);
    assert!(list_round_keys(&dropped).is_empty());
    let _ = fs::remove_dir_all(&dropped);
}

fn early_wake_does_not_play_another_round() {
    let early = support::temp_pack("early-wake");
    let mut shard = open(&early);
    let session = shard.seat_player("pc_rowan").unwrap();
    shard.queue_wait("pc_rowan").unwrap();
    commit_then_fast_forward(&mut shard).unwrap();
    let committed = shard.now().clone();
    assert_eq!(committed.world_time, 1085);
    assert!(shard.release_player_seat(&session, "pc_rowan").unwrap());
    assert!(!on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().world_time, committed.world_time);
    assert_eq!(shard.now().round_index, committed.round_index);
    assert_eq!(list_round_keys(&early), vec!["round-0001".to_string()]);
    let _ = fs::remove_dir_all(&early);
}

fn write_idle(pack: &Path, value: &str) {
    let path = pack.join("rules/tick.yaml");
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("\nidle:"), "tick.yaml already sets idle");
    let next = text.replacen(
        "schema: tick\n",
        &format!("schema: tick\nidle: {value}\n"),
        1,
    );
    assert_ne!(next, text, "tick.yaml has no schema line");
    fs::write(&path, next).unwrap();
    stamp_pack_rev(pack);
}

fn idle_off_past_due_wake_writes_nothing() {
    let pack = support::temp_pack("idle-off-due");
    write_idle(&pack, "off");
    let (world_time, state_rev, round_index, stamp) = {
        let shard = open(&pack);
        let every = shard.budget().tick_every_real_seconds;
        assert!(
            every > 0 && every + 60 < 3600,
            "interval {every} is not inside one hour"
        );
        assert!(!shard.budget().idle_ticking);
        let stamp = spookery::host::idle_clock::wall_ago(every + 60).unwrap();
        let mut now = shard.now().clone();
        let world_time = now.world_time;
        let state_rev = now.state_rev;
        let round_index = now.round_index;
        drop(shard);
        now.last_tick_at = stamp.clone();
        save_now(&pack, &now);
        (world_time, state_rev, round_index, stamp)
    };
    let mut shard = open(&pack);
    let elapsed = spookery::host::idle_clock::seconds_since(&stamp).unwrap();
    assert!(elapsed >= shard.budget().tick_every_real_seconds);
    assert!(elapsed < 3600);
    assert!(!on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().world_time, world_time);
    assert_eq!(shard.now().last_tick_at, stamp);
    assert_eq!(shard.now().state_rev, state_rev);
    assert_eq!(shard.now().round_index, round_index);
    drop(shard);
    let saved = now_of(&pack);
    assert_eq!(saved.world_time, world_time);
    assert_eq!(saved.last_tick_at, stamp);
    assert_eq!(saved.state_rev, state_rev);
    assert!(list_round_keys(&pack).is_empty());
    let _ = fs::remove_dir_all(&pack);
}

fn idle_off_long_absence_writes_no_catchup() {
    let pack = support::temp_pack("idle-off-absence");
    write_idle(&pack, "off");
    let mut shard = open(&pack);
    let world_time = shard.now().world_time;
    let last = shard.now().last_tick_at.clone();
    let state_rev = shard.now().state_rev;
    let round_index = shard.now().round_index;
    assert!(spookery::host::idle_clock::seconds_since(&last).unwrap() >= 3600);
    assert!(!on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().world_time, world_time);
    assert_eq!(shard.now().last_tick_at, last);
    assert_eq!(shard.now().round_index, round_index);
    drop(shard);
    let saved = now_of(&pack);
    assert_eq!(saved.world_time, world_time);
    assert_eq!(saved.last_tick_at, last);
    assert_eq!(saved.state_rev, state_rev);
    assert!(list_round_keys(&pack).is_empty());
    let _ = fs::remove_dir_all(&pack);
}

fn idle_off_seated_line_and_spook_tick_still_advance() {
    let pack = support::temp_pack("idle-off-play");
    write_idle(&pack, "off");
    let mut shard = open(&pack);
    shard.seat_player("pc_rowan").unwrap();
    let wall = wall_now().unwrap();
    let played = handle_player_move(&mut shard, "pc_rowan", "wait", None, false, &wall).unwrap();
    let MoveOutcome::Committed { advanced, .. } = played else {
        panic!("wait stayed queued");
    };
    assert!(advanced);
    assert_eq!(shard.now().world_time, 1085);
    assert_eq!(shard.now().round_index, 1);
    drop(shard);
    assert_eq!(list_round_keys(&pack), vec!["round-0001".to_string()]);
    let mut again = Shard::resume(&pack).unwrap_or_else(|err| panic!("resume: {err}"));
    again.seat_player("pc_rowan").unwrap();
    let wall = again.now().last_tick_at.clone();
    again
        .play(&wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
    let now = now_of(&pack);
    assert_eq!(now.round_index, 2);
    assert_eq!(now.world_time, 1090);
    assert_eq!(
        list_round_keys(&pack),
        vec!["round-0001".to_string(), "round-0002".to_string()]
    );
    let _ = fs::remove_dir_all(&pack);
}

fn idle_off_commit_round_still_commits() {
    let pack = support::temp_pack("idle-off-commit");
    write_idle(&pack, "off");
    let mut shard = open(&pack);
    shard.seat_player("pc_rowan").unwrap();
    let wall = wall_now().unwrap();
    let queued = handle_player_move(&mut shard, "pc_rowan", "wait", None, true, &wall).unwrap();
    let MoveOutcome::Queued(_) = queued else {
        panic!("wait committed with a GM seated");
    };
    assert_eq!(shard.now().world_time, 1080);
    assert!(list_round_keys(&pack).is_empty());
    let done = call_gm_tool(
        &mut shard,
        "commit_round",
        Some(json!({ "include_tick": true }).as_object().unwrap().clone()),
    );
    assert!(done.value.get("code").is_none(), "{}", done.value);
    assert!(done.ticked);
    assert_eq!(shard.now().world_time, 1085);
    assert_eq!(shard.now().round_index, 1);
    assert_eq!(list_round_keys(&pack), vec!["round-0001".to_string()]);
    let _ = fs::remove_dir_all(&pack);
}

fn a_due_wake_applies_the_queued_wait() {
    let due = support::temp_pack("due-wake");
    let mut shard = open(&due);
    let note = shard.queue_wait("pc_rowan").unwrap();
    assert_eq!(shard.now().last_tick_at, "2026-01-01T00:00:00Z");
    assert!(on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().round_index, 12);
    assert_eq!(shard.now().world_time, 1800);
    assert!(shard.queued_intents().is_empty());
    assert_eq!(
        player_waits(&round_value(&due, "round-0001")),
        vec![note.id]
    );
    assert!(player_waits(&round_value(&due, "round-0002")).is_empty());
    assert!(!on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().round_index, 12);
    let _ = fs::remove_dir_all(&due);
}

#[when(expr = "a queued wait meets a wake and the seat drops")]
fn a_queued_wait_meets_a_wake(_world: &mut World) {
    queued_line_survives_a_dropped_seat();
}

#[when(expr = "the player drops and sits again with a queued wait")]
fn the_player_drops_and_sits_again(_world: &mut World) {
    a_reseated_player_keeps_the_queued_wait();
}

#[when(expr = "a committed wait wakes early")]
fn a_committed_wait_wakes_early(_world: &mut World) {
    early_wake_does_not_play_another_round();
}

#[when(expr = "a queued wait is due")]
fn a_queued_wait_is_due(_world: &mut World) {
    a_due_wake_applies_the_queued_wait();
}

#[when(expr = "idle is off and a past-due wake arrives")]
fn idle_is_off_and_a_past_due_wake_arrives(_world: &mut World) {
    idle_off_past_due_wake_writes_nothing();
}

#[when(expr = "idle is off and a long absence wakes")]
fn idle_is_off_and_a_long_absence_wakes(_world: &mut World) {
    idle_off_long_absence_writes_no_catchup();
}

#[when(expr = "idle is off and a player waits and spook tick runs")]
fn idle_is_off_and_a_player_waits(_world: &mut World) {
    idle_off_seated_line_and_spook_tick_still_advance();
}

#[when(expr = "idle is off and the GM commits a queued wait")]
fn idle_is_off_and_the_gm_commits(_world: &mut World) {
    idle_off_commit_round_still_commits();
}
