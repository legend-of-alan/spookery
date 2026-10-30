use cucumber::when;

use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use spookery::host::idle_clock::catch_up_if_due;
use spookery::host::{call_gm_tool, commit_then_fast_forward};
use spookery_engine::testing::journal::list_round_keys;
use spookery_engine::{Presence, World as Shard};

use crate::support::{self, read_round, resume, stamp_pack_rev, state_of, write_save};
use crate::World;

use super::table;

fn player_wait_ids(round: &Value) -> Vec<String> {
    round["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .map(|intent| intent["id"].as_str().unwrap().to_string())
        .collect()
}

/// Rowan is the one seated player.
fn rowan() -> Presence {
    Presence {
        humans: 1,
        actors_present: vec!["pc_rowan".to_string()],
    }
}

fn queued_line_survives_a_dropped_seat() {
    let held = support::copy_docks("held");
    let mut shard = resume(&held);
    assert!(!catch_up_if_due(&mut shard, true, false).unwrap());
    let queued = shard.queue_wait(&rowan(), "pc_rowan").unwrap();
    assert!(!catch_up_if_due(&mut shard, true, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.queued_intents()[0].id, queued.id);
    assert_eq!(shard.state().world_time, 1080);
    assert!(list_round_keys(&held).is_empty());
    assert!(!catch_up_if_due(&mut shard, false, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.state().round_index, 0);
    assert!(list_round_keys(&held).is_empty());
    let _ = fs::remove_dir_all(&held);
}

fn early_wake_does_not_play_another_round() {
    let early = support::copy_docks("early-wake");
    let mut shard = resume(&early);
    shard.queue_wait(&rowan(), "pc_rowan").unwrap();
    commit_then_fast_forward(&mut shard, &rowan()).unwrap();
    let committed = shard.state().clone();
    assert_eq!(committed.world_time, 1085);
    assert!(!catch_up_if_due(&mut shard, false, false).unwrap());
    assert_eq!(shard.state().world_time, committed.world_time);
    assert_eq!(shard.state().round_index, committed.round_index);
    assert_eq!(list_round_keys(&early), vec!["round-0001".to_string()]);
    let _ = fs::remove_dir_all(&early);
}

fn write_idle(pack: &Path, value: &str) {
    let path = pack.join("rules/budget.yaml");
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("\nidle:"), "budget.yaml already sets idle");
    let next = text.replacen(
        "schema: budget\n",
        &format!("schema: budget\nidle: {value}\n"),
        1,
    );
    assert_ne!(next, text, "budget.yaml has no schema line");
    fs::write(&path, next).unwrap();
    stamp_pack_rev(pack);
}

fn idle_off_past_due_wake_writes_nothing() {
    let pack = support::copy_docks("idle-off-due");
    write_idle(&pack, "off");
    let (world_time, state_rev, round_index, stamp) = {
        let shard = resume(&pack);
        let every = shard.budget().round_every_real_seconds;
        assert!(
            every > 0 && every + 60 < 3600,
            "interval {every} is not inside one hour"
        );
        assert!(!shard.budget().idle_rounds);
        let stamp = spookery::host::idle_clock::wall_ago(every + 60).unwrap();
        let mut now = shard.state().clone();
        let world_time = now.world_time;
        let state_rev = now.state_rev;
        let round_index = now.round_index;
        drop(shard);
        now.last_round_at = stamp.clone();
        write_save(&pack, &now);
        (world_time, state_rev, round_index, stamp)
    };
    let mut shard = resume(&pack);
    let elapsed = spookery::host::idle_clock::secs_since(&stamp).unwrap();
    assert!(elapsed >= shard.budget().round_every_real_seconds);
    assert!(elapsed < 3600);
    assert!(!catch_up_if_due(&mut shard, false, false).unwrap());
    assert_eq!(shard.state().world_time, world_time);
    assert_eq!(shard.state().last_round_at, stamp);
    assert_eq!(shard.state().state_rev, state_rev);
    assert_eq!(shard.state().round_index, round_index);
    drop(shard);
    let saved = state_of(&pack);
    assert_eq!(saved.world_time, world_time);
    assert_eq!(saved.last_round_at, stamp);
    assert_eq!(saved.state_rev, state_rev);
    assert!(list_round_keys(&pack).is_empty());
    let _ = fs::remove_dir_all(&pack);
}

fn idle_off_long_absence_writes_no_catchup() {
    let pack = support::copy_docks("idle-off-absence");
    write_idle(&pack, "off");
    let mut shard = resume(&pack);
    let world_time = shard.state().world_time;
    let last = shard.state().last_round_at.clone();
    let state_rev = shard.state().state_rev;
    let round_index = shard.state().round_index;
    assert!(spookery::host::idle_clock::secs_since(&last).unwrap() >= 3600);
    assert!(!catch_up_if_due(&mut shard, false, false).unwrap());
    assert_eq!(shard.state().world_time, world_time);
    assert_eq!(shard.state().last_round_at, last);
    assert_eq!(shard.state().round_index, round_index);
    drop(shard);
    let saved = state_of(&pack);
    assert_eq!(saved.world_time, world_time);
    assert_eq!(saved.last_round_at, last);
    assert_eq!(saved.state_rev, state_rev);
    assert!(list_round_keys(&pack).is_empty());
    let _ = fs::remove_dir_all(&pack);
}

fn idle_off_seated_line_and_spook_round_still_advance() {
    let pack = support::copy_docks("idle-off-play");
    write_idle(&pack, "off");
    let mut shard = resume(&pack);
    shard.queue_wait(&rowan(), "pc_rowan").unwrap();
    let outputs = commit_then_fast_forward(&mut shard, &rowan()).unwrap();
    assert!(!outputs.is_empty());
    assert_eq!(shard.state().world_time, 1085);
    assert_eq!(shard.state().round_index, 1);
    drop(shard);
    assert_eq!(list_round_keys(&pack), vec!["round-0001".to_string()]);
    let lock = spookery_engine::lock_pack(&pack).unwrap();
    let mut again = Shard::resume(&pack, lock).unwrap_or_else(|err| panic!("resume: {err}"));
    let wall = again.state().last_round_at.clone();
    again
        .play(&rowan(), &wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
    let now = state_of(&pack);
    assert_eq!(now.round_index, 2);
    assert_eq!(now.world_time, 1090);
    assert_eq!(
        list_round_keys(&pack),
        vec!["round-0001".to_string(), "round-0002".to_string()]
    );
    let _ = fs::remove_dir_all(&pack);
}

fn idle_off_commit_round_still_commits() {
    let pack = support::copy_docks("idle-off-commit");
    write_idle(&pack, "off");
    let mut shard = resume(&pack);
    shard.queue_wait(&rowan(), "pc_rowan").unwrap();
    assert_eq!(shard.state().world_time, 1080);
    assert!(list_round_keys(&pack).is_empty());
    let done = call_gm_tool(
        &mut shard,
        &rowan(),
        "commit_round",
        Some(
            json!({ "include_round": true })
                .as_object()
                .unwrap()
                .clone(),
        ),
    );
    assert!(done.value.get("code").is_none(), "{}", done.value);
    assert!(!done.outputs.is_empty());
    assert_eq!(shard.state().world_time, 1085);
    assert_eq!(shard.state().round_index, 1);
    assert_eq!(list_round_keys(&pack), vec!["round-0001".to_string()]);
    let _ = fs::remove_dir_all(&pack);
}

fn a_due_wake_applies_the_queued_wait() {
    let due = support::copy_docks("due-wake");
    let mut shard = resume(&due);
    let note = shard.queue_wait(&Presence::default(), "pc_rowan").unwrap();
    assert_eq!(shard.state().last_round_at, "2026-01-01T00:00:00Z");
    assert!(catch_up_if_due(&mut shard, false, false).unwrap());
    assert_eq!(shard.state().round_index, 12);
    assert_eq!(shard.state().world_time, 1800);
    assert!(shard.queued_intents().is_empty());
    assert_eq!(
        player_wait_ids(&read_round(&due, "round-0001")),
        vec![note.id]
    );
    assert!(player_wait_ids(&read_round(&due, "round-0002")).is_empty());
    assert!(!catch_up_if_due(&mut shard, false, false).unwrap());
    assert_eq!(shard.state().round_index, 12);
    let _ = fs::remove_dir_all(&due);
}

#[when(expr = "a queued wait meets a wake and the seat drops")]
fn a_queued_wait_meets_a_wake(_world: &mut World) {
    queued_line_survives_a_dropped_seat();
}

/// With a GM seated, Rowan queues a wait, leaves, and sits again. The wait stays queued.
#[when(expr = "the player drops and sits again with a queued wait")]
fn the_player_drops_and_sits_again(world: &mut World) {
    let pack = support::copy_docks("drop-queue");
    world.pack = Some(pack.clone());
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "pc_rowan");
    let queued = table::actor_send(world, "pc_rowan", "wait", "\"kind\":\"queued\"");
    table::close_actor(world, "pc_rowan");
    table::wait_humans(world, &pack, 0);
    table::live_mut(world).players.clear();
    table::seat_actor(world, &pack, "pc_rowan");
    let status = table::gm_call(world, 4, "table_status", "{}");
    let value: Value = serde_json::from_str(&status).unwrap();
    let rows = value["result"]["structuredContent"]["queued"]
        .as_array()
        .unwrap_or_else(|| panic!("no queue\n{status}"));
    assert_eq!(rows.len(), 1, "{status}");
    assert_eq!(rows[0]["actor_id"], "pc_rowan", "{status}");
    let id = rows[0]["id"].as_str().unwrap();
    assert!(
        queued.contains(id),
        "the queue holds another wait\n{queued}\n{status}"
    );
    assert!(list_round_keys(&pack).is_empty());
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

#[when(expr = "idle is off and a player waits and spook round runs")]
fn idle_is_off_and_a_player_waits(_world: &mut World) {
    idle_off_seated_line_and_spook_round_still_advance();
}

#[when(expr = "idle is off and the GM commits a queued wait")]
fn idle_is_off_and_the_gm_commits(_world: &mut World) {
    idle_off_commit_round_still_commits();
}
