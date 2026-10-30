use cucumber::when;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use spookery_engine::testing::journal::list_round_keys;
use spookery_engine::{Presence, World as Shard};

use crate::support::{self, read_round, stamp_pack_rev, state_of};
use crate::World;

use super::table;

/// The docks pack's `round_every_real_seconds`: one idle timer interval.
const INTERVAL: Duration = Duration::from_secs(300);

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
        players: 1,
        actors_present: vec!["pc_rowan".to_string()],
    }
}

/// A docks copy served in this process, on a paused clock.
fn serve_docks(world: &mut World, name: &str, idle: Option<&str>) -> PathBuf {
    let pack = support::copy_docks(name);
    if let Some(value) = idle {
        write_idle(&pack, value);
    }
    world.pack = Some(pack.clone());
    table::listen_in_process(world, &pack);
    pack
}

/// Rowan's `wait` with a GM seated: the id of the queued line.
fn queue_rowan_wait(world: &mut World) -> String {
    let text = table::actor_send(world, "pc_rowan", "wait", "\"kind\":\"queued\"");
    let line = text
        .lines()
        .find(|line| line.contains("\"kind\":\"queued\""))
        .unwrap_or_else(|| panic!("no queued line\n{text}"));
    let queued: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}\n{line}"));
    queued["id"].as_str().unwrap().to_string()
}

/// Rowan's `wait` with no GM seated commits its own round.
fn commit_rowan_wait(world: &mut World, pack: &Path) {
    let before = list_round_keys(pack).len();
    table::actor_write(world, "pc_rowan", "wait");
    table::wait_committed_wait(pack, "pc_rowan", before);
}

/// Advance one interval, then read status so the wake it sent has run.
fn wake(world: &mut World, by: Duration) {
    table::advance(world, by);
    let _ = table::read_presence(world);
}

fn queued_line_survives_a_dropped_seat(world: &mut World) {
    let pack = serve_docks(world, "held", None);
    table::seat_rowan(world, &pack);
    wake(world, INTERVAL);
    table::release_probe_gm(world);
    table::seat_gm(world);
    let id = queue_rowan_wait(world);
    wake(world, INTERVAL);
    table::close_actor(world, "pc_rowan");
    table::wait_players(world, &pack, 0);
    wake(world, INTERVAL);
    let status = table::gm_call(world, 4, "table_status", "{}");
    let value: Value = serde_json::from_str(&status).unwrap();
    let rows = value["result"]["structuredContent"]["queued"]
        .as_array()
        .unwrap_or_else(|| panic!("no queue\n{status}"));
    assert_eq!(rows.len(), 1, "{status}");
    assert_eq!(rows[0]["id"], id.as_str(), "{status}");
    assert_eq!(table::now_of(&pack).world_time, 1080);
    assert!(list_round_keys(&pack).is_empty());
}

fn early_wake_does_not_play_another_round(world: &mut World) {
    let pack = serve_docks(world, "early-wake", None);
    table::seat_rowan(world, &pack);
    commit_rowan_wait(world, &pack);
    let committed = table::now_of(&pack);
    assert_eq!(committed.world_time, 1085);
    table::close_actor(world, "pc_rowan");
    table::wait_players(world, &pack, 0);
    table::release_probe_gm(world);
    // The timer fires a full interval later, but the save's stamp is seconds old.
    wake(world, INTERVAL);
    let now = table::now_of(&pack);
    assert_eq!(now.world_time, committed.world_time);
    assert_eq!(now.round_index, committed.round_index);
    assert_eq!(list_round_keys(&pack), vec!["round-0001".to_string()]);
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

/// With idle off no timer is armed, so an empty table writes nothing however
/// long it sits. The docks save's stamp is already past due.
fn idle_off_empty_table_writes_nothing(world: &mut World, name: &str, by: Duration) {
    let pack = serve_docks(world, name, Some("off"));
    wake(world, by);
    let opened = table::live_mut(world).opened_save.clone();
    let now = fs::read(pack.join("state/save.yaml")).unwrap();
    assert_eq!(now, opened, "the save changed");
    assert!(list_round_keys(&pack).is_empty());
}

fn idle_off_seated_line_and_spook_round_still_advance(world: &mut World) {
    let pack = serve_docks(world, "idle-off-play", Some("off"));
    table::seat_rowan(world, &pack);
    commit_rowan_wait(world, &pack);
    table::stop_host(world);
    let now = state_of(&pack);
    assert_eq!(now.world_time, 1085);
    assert_eq!(now.round_index, 1);
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
}

fn idle_off_commit_round_still_commits(world: &mut World) {
    let pack = serve_docks(world, "idle-off-commit", Some("off"));
    table::seat_gm(world);
    table::seat_rowan(world, &pack);
    let _ = queue_rowan_wait(world);
    assert!(list_round_keys(&pack).is_empty());
    let done = table::gm_call(world, 5, "commit_round", r#"{"include_round":true}"#);
    let value: Value = serde_json::from_str(&done).unwrap();
    let body = &value["result"]["structuredContent"];
    assert!(body.get("code").is_none(), "{done}");
    let now = table::now_of(&pack);
    assert_eq!(now.world_time, 1085);
    assert_eq!(now.round_index, 1);
    assert_eq!(list_round_keys(&pack), vec!["round-0001".to_string()]);
}

fn a_due_wake_applies_the_queued_wait(world: &mut World) {
    let pack = serve_docks(world, "due-wake", None);
    assert_eq!(table::now_of(&pack).last_round_at, "2026-01-01T00:00:00Z");
    table::seat_gm(world);
    table::seat_rowan(world, &pack);
    let id = queue_rowan_wait(world);
    table::close_actor(world, "pc_rowan");
    table::wait_players(world, &pack, 0);
    let _ = table::release_gm(world);
    wake(world, INTERVAL);
    let now = table::now_of(&pack);
    assert_eq!(now.round_index, 12);
    assert_eq!(now.world_time, 1800);
    assert_eq!(player_wait_ids(&read_round(&pack, "round-0001")), vec![id]);
    assert!(player_wait_ids(&read_round(&pack, "round-0002")).is_empty());
    // The catch-up stamped the save, so the next wake is not due.
    table::release_probe_gm(world);
    wake(world, INTERVAL);
    assert_eq!(table::now_of(&pack).round_index, 12);
}

#[when(expr = "a queued wait meets a wake and the seat drops")]
fn a_queued_wait_meets_a_wake(world: &mut World) {
    queued_line_survives_a_dropped_seat(world);
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
    table::wait_players(world, &pack, 0);
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
fn a_committed_wait_wakes_early(world: &mut World) {
    early_wake_does_not_play_another_round(world);
}

#[when(expr = "a queued wait is due")]
fn a_queued_wait_is_due(world: &mut World) {
    a_due_wake_applies_the_queued_wait(world);
}

#[when(expr = "idle is off and a past-due wake arrives")]
fn idle_is_off_and_a_past_due_wake_arrives(world: &mut World) {
    idle_off_empty_table_writes_nothing(world, "idle-off-due", Duration::from_secs(360));
}

#[when(expr = "idle is off and a long absence wakes")]
fn idle_is_off_and_a_long_absence_wakes(world: &mut World) {
    idle_off_empty_table_writes_nothing(world, "idle-off-absence", Duration::from_secs(3700));
}

#[when(expr = "idle is off and a player waits and spook round runs")]
fn idle_is_off_and_a_player_waits(world: &mut World) {
    idle_off_seated_line_and_spook_round_still_advance(world);
}

#[when(expr = "idle is off and the GM commits a queued wait")]
fn idle_is_off_and_the_gm_commits(world: &mut World) {
    idle_off_commit_round_still_commits(world);
}
