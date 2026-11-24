use cucumber::when;
use spookery_engine::testing::{act_of, world_of};

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use spookery_engine::testing::journal::list_round_keys;

use crate::support::{self, stamp_pack_rev};
use crate::World;

use super::host;

/// The docks pack's `round_every_real_seconds`: one idle timer interval.
const INTERVAL: Duration = Duration::from_secs(300);

/// A docks copy served in this process, on a paused clock.
async fn serve_docks(world: &mut World, label: &str, idle: Option<&str>) -> PathBuf {
    let pack = support::copy_pack("docks", label);
    if let Some(value) = idle {
        write_idle(&pack, value);
    }
    world.pack = Some(pack.clone());
    host::serve_in_process(world, &pack).await;
    pack
}

/// Rowan's `wait` with a GM seated: the id of the queued line.
async fn queue_rowan_wait(world: &mut World) -> String {
    let text = host::actor_send(world, "rowan", "wait", "\"kind\":\"queued\"").await;
    let line = text
        .lines()
        .find(|line| line.contains("\"kind\":\"queued\""))
        .unwrap_or_else(|| panic!("no queued line\n{text}"));
    let queued: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}\n{line}"));
    queued["body"]["intent"].as_str().unwrap().to_string()
}

/// Rowan's `wait` with no GM seated commits its own round.
async fn commit_rowan_wait(world: &mut World, pack: &Path) {
    let before = list_round_keys(pack).len();
    host::actor_write(world, "rowan", "wait");
    host::wait_committed_wait(pack, "rowan", before).await;
}

/// Advance one interval, then read status so the wake it sent has run.
async fn advance_and_settle(world: &mut World, by: Duration) {
    host::advance(world, by).await;
    let _ = host::read_presence(world).await;
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

/// With a GM seated, Rowan queues a wait. Wakes come while Rowan drops and claims again.
#[when(expr = "a queued wait meets wakes while the player drops and claims again")]
async fn a_queued_wait_meets_wakes(world: &mut World) {
    let pack = serve_docks(world, "held", None).await;
    host::seat_rowan(world, &pack).await;
    advance_and_settle(world, INTERVAL).await;
    host::release_probe_gm(world).await;
    host::seat_gm(world).await;
    let id = queue_rowan_wait(world).await;
    advance_and_settle(world, INTERVAL).await;
    host::close_actor(world, "rowan").await;
    host::wait_players(world, 0).await;
    advance_and_settle(world, INTERVAL).await;
    host::live_mut(world).players.clear();
    host::seat_rowan(world, &pack).await;
    let status = host::gm_tool_call(world, 4, "status", "{}").await;
    let reply = host::read_reply(&status);
    let rows = reply["queued"]
        .as_array()
        .unwrap_or_else(|| panic!("no queue\n{status}"));
    assert_eq!(rows.len(), 1, "{status}");
    assert_eq!(rows[0]["id"], id.as_str(), "{status}");
    assert_eq!(rows[0]["actor"], "rowan", "{status}");
    assert_eq!(world_of(&pack).world_time, 1080);
}

#[when(expr = "a committed wait wakes early")]
async fn a_committed_wait_wakes_early(world: &mut World) {
    let pack = serve_docks(world, "early-wake", None).await;
    host::seat_rowan(world, &pack).await;
    commit_rowan_wait(world, &pack).await;
    let committed = world_of(&pack);
    assert_eq!(committed.world_time, 1085);
    host::close_actor(world, "rowan").await;
    host::wait_players(world, 0).await;
    host::release_probe_gm(world).await;
    advance_and_settle(world, INTERVAL).await;
    let now = world_of(&pack);
    assert_eq!(now.world_time, committed.world_time);
    assert_eq!(now.round_index, committed.round_index);
}

#[when(expr = "a queued wait is due")]
async fn a_queued_wait_is_due(world: &mut World) {
    let pack = serve_docks(world, "due-wake", None).await;
    assert_eq!(world_of(&pack).last_round_at, "2026-01-01T00:00:00Z");
    host::seat_gm(world).await;
    host::seat_rowan(world, &pack).await;
    let _ = queue_rowan_wait(world).await;
    host::close_actor(world, "rowan").await;
    host::wait_players(world, 0).await;
    let _ = host::release_gm(world).await;
    advance_and_settle(world, INTERVAL).await;
    let now = world_of(&pack);
    assert_eq!(now.round_index, 12);
    assert_eq!(now.world_time, 1800);
    assert_eq!(act_of(&now, "rowan").last_act_round, 1);
    host::release_probe_gm(world).await;
    advance_and_settle(world, INTERVAL).await;
    assert_eq!(world_of(&pack).round_index, 12);
}

/// With idle off no timer is armed, so an empty host writes nothing however
/// long it stays idle. The docks save's stamp is already past due.
#[when(expr = "idle is off and an empty table wakes after {int} seconds")]
async fn idle_is_off_and_an_empty_table_wakes(world: &mut World, seconds: u64) {
    let pack = serve_docks(world, "idle-off-empty", Some("off")).await;
    advance_and_settle(world, Duration::from_secs(seconds)).await;
    let opened = host::live_mut(world).opened_save.clone();
    let now = fs::read(pack.join("state/save.yaml")).unwrap();
    assert_eq!(now, opened, "the save changed");
}

#[when(expr = "idle is off and a player waits and spook round runs")]
async fn idle_is_off_and_a_player_waits(world: &mut World) {
    let pack = serve_docks(world, "idle-off-play", Some("off")).await;
    host::seat_rowan(world, &pack).await;
    commit_rowan_wait(world, &pack).await;
    host::stop(world);
    let now = world_of(&pack);
    assert_eq!(now.world_time, 1085);
    assert_eq!(now.round_index, 1);
    host::run(world, &["round", pack.to_str().unwrap()]).await;
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    let now = world_of(&pack);
    assert_eq!(now.round_index, 2);
    assert_eq!(now.world_time, 1145);
}

#[when(expr = "idle is off and the GM commits a queued wait")]
async fn idle_is_off_and_the_gm_commits(world: &mut World) {
    let pack = serve_docks(world, "idle-off-commit", Some("off")).await;
    host::seat_gm(world).await;
    host::seat_rowan(world, &pack).await;
    let _ = queue_rowan_wait(world).await;
    assert_eq!(world_of(&pack).round_index, 0);
    let done = host::gm_tool_call(world, 5, "commit", r#"{"include_round":true}"#).await;
    assert_eq!(host::read_reply(&done)["kind"], "committed", "{done}");
    let now = world_of(&pack);
    assert_eq!(now.world_time, 1085);
    assert_eq!(now.round_index, 1);
}
