//! The docks yard: a strike fight through play, and the way onto the lane and its ending.

use spookery_engine::testing::journal::list_round_keys;
use spookery_engine::testing::{place_of, state_of};
use spookery_engine::{component, entity};
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;

use super::common::wait_play_until;
use super::table;
use crate::support;
use crate::World;

/// The way onto the lane: Briar steps aside, and the chock holds the cart's wheel.
const SCRIPT: &[&str] = &[
    "ask briar",
    "travel wheel_shed",
    "take chock",
    "travel gate_yard",
    "chock",
    "travel mill_lane",
];

#[when(expr = "the save before the fight is kept")]
fn the_save_before_the_fight_is_kept(world: &mut World) {
    let dest = support::fresh_dir("before");
    support::copy_dir(table::pack(world), &dest);
    world.scratch = dest.display().to_string();
}

#[then(expr = "play shows {int} successful rolls")]
fn play_shows_successful_rolls(world: &mut World, rolls: usize) {
    let succeeded = |stdout: &str| stdout.matches("Your roll succeeded.").count() >= rolls;
    wait_play_until(world, succeeded, &format!("{rolls} successful rolls"));
}

#[then(expr = "briar's hp is {int}")]
fn briars_hp_is(world: &mut World, expected: i64) {
    let pack = table::pack(world).to_path_buf();
    let briar = entity::Id::Actor("briar".into());
    table::poll_until(&format!("briar hp {expected}"), || {
        let now = state_of(&pack);
        match now.entities().get::<component::Meter>(&briar).value("hp") {
            Ok(hp) if hp == expected => Ok(()),
            held => Err(format!("hp {held:?}")),
        }
    });
}

#[when(expr = "the yard script is played with the stub GM")]
fn the_yard_script_is_played_with_the_stub_gm(world: &mut World) {
    let pack = support::copy_pack("docks", "yard-script");
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "harbormouse");
    world.pack = Some(pack.clone());
    let (lane, way) = SCRIPT.split_last().unwrap();
    for line in way {
        commit_line(world, line);
    }
    world.start_ticks = state_of(&pack).world_time;
    commit_line(world, lane);
}

#[then(expr = "Harbormouse reaches the lane at the exit's cost in ticks and hears the ending")]
fn harbormouse_reaches_the_lane_and_hears_the_ending(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let cost = lane_cost(&pack);
    assert!(cost > 0);
    assert_eq!(place_of(&state_of(&pack), "harbormouse"), "mill_lane");
    assert_eq!(arrival_tick(&pack).saturating_sub(world.start_ticks), cost);
    assert!(
        read_rounds(&pack).iter().any(|r| r["fast_forward"] == true),
        "the journey did not fast-forward"
    );
    let flags = state_of(&pack);
    let flags = flags.entities().get::<component::Flag>(&entity::Id::Global);
    assert!(flags.contains("finished"), "finished is not set");
    table::wait_player_from(
        world,
        0,
        "The cart rolls out onto the lane. That is the end.",
    );
}

#[then(expr = "spook verify passes on the save")]
fn verify_passes_on_the_save(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::stop_host(world);
    let verified = support::run_spook(&["verify", pack.to_str().unwrap()]);
    let verify_out = String::from_utf8_lossy(&verified.stdout);
    let verify_err = String::from_utf8_lossy(&verified.stderr);
    assert!(
        verified.status.success(),
        "verify failed\n{verify_out}\n{verify_err}"
    );
    assert!(verify_out.contains("verified"), "{verify_out}");
}

#[given(expr = "the stub GM attempts to set gold directly")]
fn the_stub_gm_attempts_to_set_gold(world: &mut World) {
    let pack = support::copy_pack("docks", "yard-gold");
    table::listen(world, &pack);
    table::seat_gm(world);
    world.pack = Some(pack);
    world.stdout = table::gm_tool_call(world, 4, "set_gold", r#"{"gold":5}"#);
}

#[then(expr = "the attempt is refused and the refusal is in the log")]
fn the_attempt_is_refused_and_in_the_log(world: &mut World) {
    assert_eq!(
        table::read_reply(&world.stdout)["code"],
        "SCHEMA",
        "{}",
        world.stdout
    );
    table::wait_host_until(world, "the refusal in the host log", |log| {
        log.contains("refused set_gold SCHEMA")
    });
    let saved = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save: {err}"));
    assert_eq!(saved, table::live_mut(world).opened_save);
}

/// The player sends `line`, then the GM commits its round.
fn commit_line(world: &mut World, line: &str) {
    let mark = table::player_write(world, line);
    table::wait_player_from(world, mark, "\"kind\":\"queued\"");
    let before = list_round_keys(table::pack(world)).len();
    table::gm_tool_call(world, 2, "commit", r#"{"include_round":true}"#);
    table::wait_round_count(table::pack(world), before + 1);
}

fn lane_cost(pack: &Path) -> u64 {
    let canon =
        spookery_engine::load_canon(pack).unwrap_or_else(|err| panic!("load canon: {err:?}"));
    let yard = canon.places.get("gate_yard").unwrap();
    let lane = yard.exits.iter().find(|exit| exit.name == "mill_lane");
    lane.unwrap().cost.ticks
}

fn arrival_tick(pack: &Path) -> u64 {
    read_rounds(pack)
        .into_iter()
        .flat_map(|round| round["effects"].as_array().cloned().unwrap_or_default())
        .find(|effect| effect["kind"] == "location" && effect["to"] == "mill_lane")
        .unwrap_or_else(|| panic!("no arrival on mill lane"))["arrived_at"]
        .as_u64()
        .unwrap()
}

fn read_rounds(pack: &Path) -> Vec<Value> {
    list_round_keys(pack)
        .into_iter()
        .map(|name| {
            let text = spookery_engine::testing::journal::read_record_line(pack, &name)
                .unwrap_or_else(|err| panic!("read {name}: {}", err.detail));
            serde_json::from_str(&text).unwrap_or_else(|err| panic!("{name}: {err}"))
        })
        .collect()
}
