//! The sample-d20 arc: three ways onto a timed lane, then an ending.

use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::journal::list_tick_keys;

use super::table;
use crate::support;
use crate::World;

const EVENING: u64 = 1080;
const OPENING: &[&str] = &["strike npc_briar", "use tonic"];
const SCRIPT: &[&str] = &[
    "strike npc_briar",
    "use tonic",
    "travel loft",
    "travel gate_yard",
    "ask_briar",
    "travel wheel_shed",
    "take chock",
    "travel gate_yard",
    "chock",
    "travel mill_lane",
];
const BRANCHES: &[&[&str]] = &[
    &[
        "ask_briar",
        "travel wheel_shed",
        "take chock",
        "travel gate_yard",
        "chock",
        "travel mill_lane",
    ],
    &[
        "strike npc_briar",
        "strike npc_briar",
        "strike npc_briar",
        "strike npc_briar",
        "strike npc_briar",
        "travel wheel_shed",
        "take chock",
        "travel gate_yard",
        "chock",
        "travel mill_lane",
    ],
    &[
        "travel loft",
        "take latch",
        "travel gate_yard",
        "travel wheel_shed",
        "take chock",
        "travel gate_yard",
        "chock",
        "travel mill_lane",
    ],
];

#[given(expr = "sample-d20 and a recorded line script")]
fn sample_d20_and_a_recorded_line_script(world: &mut World) {
    world.scratch = SCRIPT.join("\n");
}

#[when(expr = "the script is played with the stub GM")]
fn the_script_is_played_with_the_stub_gm(world: &mut World) {
    let lines: Vec<String> = world.scratch.lines().map(str::to_string).collect();
    let pack = sit(world, "showcase-script");
    for (index, line) in lines.iter().enumerate() {
        commit_line(world, line);
        if index + 1 == OPENING.len() {
            let now = table::now_of(&pack);
            assert!(
                now.world_time <= EVENING + 10,
                "the opening ran past 10 minutes at {}",
                now.world_time
            );
            assert_eq!(tonic_of(&now), 0, "the tonic is still in inventory");
            assert!(
                table::player_text(world).contains("\"kind\":\"check\""),
                "the opening roll was not shown\n{}",
                table::player_text(world)
            );
        }
    }
    world.pack = Some(pack);
}

#[then(expr = "the ending flag is set within the session")]
fn the_ending_flag_is_set(world: &mut World) {
    assert!(finished(table::pack(world)), "finished is not set");
}

#[then(expr = "spook verify passes on the save")]
fn verify_passes_on_the_save(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::stop_server(world);
    prove(&pack);
}

#[then(expr = "each of the 2-3 branch scripts reaches an ending")]
fn each_branch_reaches_an_ending(world: &mut World) {
    for (index, lines) in BRANCHES.iter().enumerate() {
        let pack = sit(world, &format!("showcase-branch-{index}"));
        for line in *lines {
            commit_line(world, line);
        }
        assert!(finished(&pack), "branch {index} did not finish: {lines:?}");
        table::stop_server(world);
        prove(&pack);
    }
}

#[when(expr = "the party travels through the locked exit after unlocking it")]
fn the_party_travels_through_the_locked_exit(world: &mut World) {
    let pack = sit(world, "showcase-journey");
    commit_line(world, "ask_briar");
    commit_line(world, "travel wheel_shed");
    commit_line(world, "take chock");
    commit_line(world, "travel gate_yard");
    commit_line(world, "chock");
    let started = table::now_of(&pack).world_time;
    commit_line(world, "travel mill_lane");
    world.scratch = started.to_string();
    world.pack = Some(pack);
}

#[then(expr = "they arrive at the exit's cost in minutes and the arrival beat fires")]
fn they_arrive_at_the_exits_cost(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let started: u64 = world.scratch.parse().unwrap();
    let cost = lane_cost(&pack);
    let now = table::now_of(&pack);
    let arrived = arrival_minute(&pack);
    assert_eq!(now.actors.get("pc_lark").unwrap().place(), "mill_lane");
    assert_eq!(arrived.saturating_sub(started), cost);
    assert!(cost > 0);
    assert!(finished(&pack));
    assert!(
        ticks(&pack).iter().any(|tick| tick["fast_forward"] == true),
        "the journey did not fast-forward"
    );
    assert!(
        table::player_text(world).contains("Oden and Nell are on the lane. That is the end."),
        "the arrival line was not shown\n{}",
        table::player_text(world)
    );
}

#[when(expr = "a player uses the consumable")]
fn a_player_uses_the_consumable(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "showcase-tonic");
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_lark");
    let (_mark, added) = table::player_send(world, "use tonic", "tonic");
    world.player_added = added;
    world.pack = Some(pack);
}

#[then(expr = "it leaves their inventory")]
fn it_leaves_their_inventory(world: &mut World) {
    let now = table::now_of(table::pack(world));
    assert_eq!(tonic_of(&now), 0, "{}", world.player_added);
}

#[given(expr = "the stub GM attempts to set gold directly")]
fn the_stub_gm_attempts_to_set_gold(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "showcase-gold");
    table::listen(world, &pack);
    table::seat_gm(world);
    world.pack = Some(pack);
    world.stdout = table::gm_call(world, 4, "set_gold", r#"{"gold":5}"#);
}

#[then(expr = "the attempt is refused and the refusal is in the log")]
fn the_attempt_is_refused_and_in_the_log(world: &mut World) {
    assert!(world.stdout.contains("NOT_IN_SPEC"), "{}", world.stdout);
    table::wait_host_until(world, "the refusal in the host log", |log| {
        log.contains("refused set_gold NOT_IN_SPEC")
    });
    let saved = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save: {err}"));
    assert_eq!(saved, table::live_mut(world).opened_now);
}

fn sit(world: &mut World, label: &str) -> PathBuf {
    let pack = support::copy_pack("sample-d20", label);
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "pc_lark");
    world.pack = Some(pack.clone());
    pack
}

fn commit_line(world: &mut World, line: &str) {
    let mark = table::player_write(world, line);
    table::wait_player_from(world, mark, "\"kind\":\"queued\"");
    let before = list_tick_keys(table::pack(world)).len();
    table::gm_call(world, 2, "commit_round", r#"{"include_tick":true}"#);
    table::wait_tick_count(table::pack(world), before + 1);
}

fn tonic_of(now: &spookery_engine::model::Now) -> u32 {
    now.actors.get("pc_lark").unwrap().count("tonic")
}

fn finished(pack: &Path) -> bool {
    table::now_of(pack)
        .flags
        .iter()
        .any(|flag| flag == "finished")
}

fn lane_cost(pack: &Path) -> u64 {
    let canon = spookery_engine::canon::load_canon(pack)
        .unwrap_or_else(|err| panic!("load canon: {err:?}"));
    canon
        .places
        .get("gate_yard")
        .unwrap()
        .exits
        .iter()
        .find(|exit| exit.name == "mill_lane")
        .unwrap()
        .minutes
}

fn arrival_minute(pack: &Path) -> u64 {
    ticks(pack)
        .into_iter()
        .flat_map(|tick| tick["arrivals"].as_array().cloned().unwrap_or_default())
        .find(|arrival| arrival["to"] == "mill_lane")
        .unwrap_or_else(|| panic!("no arrival on mill lane"))["arrived_at"]
        .as_u64()
        .unwrap()
}

fn ticks(pack: &Path) -> Vec<Value> {
    list_tick_keys(pack)
        .into_iter()
        .map(|name| {
            let text = spookery_engine::testing::journal::read_record_line(pack, &name)
                .unwrap_or_else(|err| panic!("read {name}: {}", err.detail));
            serde_json::from_str(&text).unwrap_or_else(|err| panic!("{name}: {err}"))
        })
        .collect()
}

fn prove(pack: &Path) {
    let arg = pack.to_str().unwrap();
    let verified = support::spook(&["verify", arg]);
    let verify_out = String::from_utf8_lossy(&verified.stdout);
    let verify_err = String::from_utf8_lossy(&verified.stderr);
    assert!(
        verified.status.success(),
        "verify failed\n{verify_out}\n{verify_err}"
    );
    assert!(verify_out.contains("verified"), "{verify_out}");
}
