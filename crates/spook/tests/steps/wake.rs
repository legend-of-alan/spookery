use cucumber::when;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use spookery_engine::testing::journal::list_round_keys;

use crate::support::{self, read_round, stamp_pack_rev};
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

/// A docks copy served in this process, on a paused clock.
fn serve_docks(world: &mut World, name: &str, idle: Option<&str>) -> PathBuf {
    let pack = support::copy_docks(name);
    if let Some(value) = idle {
        write_idle(&pack, value);
    }
    world.pack = Some(pack.clone());
    table::serve_in_process(world, &pack);
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
fn advance_and_settle(world: &mut World, by: Duration) {
    table::advance(world, by);
    let _ = table::read_presence(world);
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

/// With a GM seated, Rowan queues a wait. Wakes come while Rowan drops and sits again.
#[when(expr = "a queued wait meets wakes while the player drops and sits again")]
fn a_queued_wait_meets_wakes(world: &mut World) {
    let pack = serve_docks(world, "held", None);
    table::seat_rowan(world, &pack);
    advance_and_settle(world, INTERVAL);
    table::release_probe_gm(world);
    table::seat_gm(world);
    let id = queue_rowan_wait(world);
    advance_and_settle(world, INTERVAL);
    table::close_actor(world, "pc_rowan");
    table::wait_players(world, 0);
    advance_and_settle(world, INTERVAL);
    table::live_mut(world).players.clear();
    table::seat_rowan(world, &pack);
    let status = table::gm_call(world, 4, "table_status", "{}");
    let value: Value = serde_json::from_str(&status).unwrap();
    let rows = value["result"]["structuredContent"]["queued"]
        .as_array()
        .unwrap_or_else(|| panic!("no queue\n{status}"));
    assert_eq!(rows.len(), 1, "{status}");
    assert_eq!(rows[0]["id"], id.as_str(), "{status}");
    assert_eq!(rows[0]["actor_id"], "pc_rowan", "{status}");
    assert_eq!(table::now_of(&pack).world_time, 1080);
    assert!(list_round_keys(&pack).is_empty());
}

#[when(expr = "a committed wait wakes early")]
fn a_committed_wait_wakes_early(world: &mut World) {
    let pack = serve_docks(world, "early-wake", None);
    table::seat_rowan(world, &pack);
    commit_rowan_wait(world, &pack);
    let committed = table::now_of(&pack);
    assert_eq!(committed.world_time, 1085);
    table::close_actor(world, "pc_rowan");
    table::wait_players(world, 0);
    table::release_probe_gm(world);
    // The timer fires a full interval later, but the save's stamp is seconds old.
    advance_and_settle(world, INTERVAL);
    let now = table::now_of(&pack);
    assert_eq!(now.world_time, committed.world_time);
    assert_eq!(now.round_index, committed.round_index);
    assert_eq!(list_round_keys(&pack), vec!["round-0001".to_string()]);
}

#[when(expr = "a queued wait is due")]
fn a_queued_wait_is_due(world: &mut World) {
    let pack = serve_docks(world, "due-wake", None);
    assert_eq!(table::now_of(&pack).last_round_at, "2026-01-01T00:00:00Z");
    table::seat_gm(world);
    table::seat_rowan(world, &pack);
    let id = queue_rowan_wait(world);
    table::close_actor(world, "pc_rowan");
    table::wait_players(world, 0);
    let _ = table::release_gm(world);
    advance_and_settle(world, INTERVAL);
    let now = table::now_of(&pack);
    assert_eq!(now.round_index, 12);
    assert_eq!(now.world_time, 1800);
    assert_eq!(player_wait_ids(&read_round(&pack, "round-0001")), vec![id]);
    assert!(player_wait_ids(&read_round(&pack, "round-0002")).is_empty());
    // The catch-up stamped the save, so the next wake is not due.
    table::release_probe_gm(world);
    advance_and_settle(world, INTERVAL);
    assert_eq!(table::now_of(&pack).round_index, 12);
}

/// With idle off no timer is armed, so an empty table writes nothing however
/// long it sits. The docks save's stamp is already past due.
#[when(expr = "idle is off and an empty table wakes after {int} seconds")]
fn idle_is_off_and_an_empty_table_wakes(world: &mut World, seconds: u64) {
    let pack = serve_docks(world, "idle-off-empty", Some("off"));
    advance_and_settle(world, Duration::from_secs(seconds));
    let opened = table::live_mut(world).opened_save.clone();
    let now = fs::read(pack.join("state/save.yaml")).unwrap();
    assert_eq!(now, opened, "the save changed");
    assert!(list_round_keys(&pack).is_empty());
}

#[when(expr = "idle is off and a player waits and spook round runs")]
fn idle_is_off_and_a_player_waits(world: &mut World) {
    let pack = serve_docks(world, "idle-off-play", Some("off"));
    table::seat_rowan(world, &pack);
    commit_rowan_wait(world, &pack);
    table::stop_host(world);
    let now = table::now_of(&pack);
    assert_eq!(now.world_time, 1085);
    assert_eq!(now.round_index, 1);
    assert_eq!(list_round_keys(&pack), vec!["round-0001".to_string()]);
    table::run(world, &["round", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    // `spook round` plays at an empty table: one idle round of 60 minutes.
    let now = table::now_of(&pack);
    assert_eq!(now.round_index, 2);
    assert_eq!(now.world_time, 1145);
    assert_eq!(
        list_round_keys(&pack),
        vec!["round-0001".to_string(), "round-0002".to_string()]
    );
}

#[when(expr = "idle is off and the GM commits a queued wait")]
fn idle_is_off_and_the_gm_commits(world: &mut World) {
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
