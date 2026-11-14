//! The sample-d20 and demo-d20 arcs: ways onto a timed lane, then an ending.

use spookery_engine::testing::{place_of, state_of};
use spookery_engine::{component, entity};
use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::journal::list_round_keys;

use super::table;
use crate::support;
use crate::World;

const START_TIME: u64 = 1080;
const OPENING: &[&str] = &["strike briar", "use tonic"];
const SCRIPT: &[&str] = &[
    "strike briar",
    "use tonic",
    "travel loft",
    "travel gate_yard",
    "ask briar",
    "travel wheel_shed",
    "take chock",
    "travel gate_yard",
    "chock",
    "travel mill_lane",
];
const BRANCHES: &[&[&str]] = &[
    &[
        "ask briar",
        "travel wheel_shed",
        "take chock",
        "travel gate_yard",
        "chock",
        "travel mill_lane",
    ],
    &[
        "strike briar",
        "strike briar",
        "strike briar",
        "strike briar",
        "strike briar",
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

// The demo scripts. A roll's dice come from the save's seed and the round index, and
// `copy_pack` pins the seed to the world id, so each line lands on the same rolls every run:
// these are the pinned seeds the outcomes below rely on.
const DEMO_GATE: &[&str] = &[
    "travel shed",
    "take cudgel",
    "wield cudgel",
    "travel yard",
    "ask briar",
    "strike briar",
    "strike briar",
    "strike briar",
    "use tonic",
    "strike briar",
    "strike briar",
    "strike briar",
    "travel gate",
];
const DEMO_WARD: &[&str] = &[
    "travel loft",
    "banish goblin",
    "banish goblin",
    "banish goblin",
    "use tonic",
    "banish goblin",
    "banish goblin",
    "take nail",
    "travel yard",
    "ward",
    "travel cart",
];
const DEMO_BRACE: &[&str] = &[
    "travel loft",
    "banish goblin",
    "banish goblin",
    "banish goblin",
    "use tonic",
    "banish goblin",
    "banish goblin",
    "take nail",
    "travel yard",
    "travel shed",
    "brace",
    "travel yard",
    "travel cart",
];
const DEMO_KNOCKED_OUT: &[&str] = &[
    "travel shed",
    "travel yard",
    "travel loft",
    "banish goblin",
    "banish goblin",
    "banish goblin",
    "banish goblin",
    "banish goblin",
];
// A line that starts with `GM queues ` is the GM queueing that actor's move, not the player.
const DEMO_GOBLIN_BITE: &[&str] = &[
    "travel shed",
    "travel yard",
    "travel loft",
    "banish goblin",
    "banish goblin",
    "banish goblin",
    "GM queues goblin bite",
];
// The brace script up to the brace, then six more walks until the lamps are lit, and one back
// to the yard.
const DEMO_DUSK: &[&str] = &[
    "travel loft",
    "banish goblin",
    "banish goblin",
    "banish goblin",
    "use tonic",
    "banish goblin",
    "banish goblin",
    "take nail",
    "travel yard",
    "travel shed",
    "brace",
    "travel yard",
    "travel loft",
    "travel yard",
    "travel shed",
    "travel yard",
    "travel loft",
    "travel yard",
];

const DEMO_ENDINGS: &[(&str, &str)] = &[
    ("gate", "Briar sits down hard in the mud and waves you through."),
    ("warded", "The cart rolls. The nail stays in the wheel for good."),
    (
        "braced",
        "The cart rolls, and the nail is still in your pocket. Oden will remember.",
    ),
    (
        "knocked-out",
        "You go down in the mud. When you wake, the lamps are lit and the lane is shut. Nell goes home without her flour. The end.",
    ),
    (
        "dusk",
        "The lamps are lit. Briar bars the gate, and the cart won't move in the dark. The end.",
    ),
];

#[given(expr = "sample-d20 and a recorded line script")]
fn sample_d20_and_a_recorded_line_script(world: &mut World) {
    world.scratch = SCRIPT.join("\n");
}

#[when(expr = "the script is played with the stub GM")]
fn the_script_is_played_with_the_stub_gm(world: &mut World) {
    let lines: Vec<String> = world.scratch.lines().map(str::to_string).collect();
    let pack = seat_gm_and_lark(world, "showcase-script");
    for (index, line) in lines.iter().enumerate() {
        commit_line(world, line);
        if index + 1 == OPENING.len() {
            let now = state_of(&pack);
            assert!(
                now.world_time <= START_TIME + 10,
                "the opening ran past 10 ticks at {}",
                now.world_time
            );
            assert_eq!(larks_tonic(&now), 0, "the tonic is still in inventory");
            table::wait_player_from(world, 0, "\"kind\":\"check\"");
        }
    }
    world.pack = Some(pack);
}

#[then(expr = "the ending flag is set within the session")]
fn the_ending_flag_is_set(world: &mut World) {
    assert!(is_finished(table::pack(world)), "finished is not set");
}

#[then(expr = "spook verify passes on the save")]
fn verify_passes_on_the_save(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::stop_host(world);
    assert_verifies(&pack);
}

#[then(expr = "each of the 2-3 branch scripts reaches an ending")]
fn each_branch_reaches_an_ending(world: &mut World) {
    for (index, lines) in BRANCHES.iter().enumerate() {
        let pack = seat_gm_and_lark(world, &format!("showcase-branch-{index}"));
        for line in *lines {
            commit_line(world, line);
        }
        assert!(
            is_finished(&pack),
            "branch {index} did not finish: {lines:?}"
        );
        table::stop_host(world);
        assert_verifies(&pack);
    }
}

#[when(expr = "the party travels through the locked exit after unlocking it")]
fn the_party_travels_through_the_locked_exit(world: &mut World) {
    let pack = seat_gm_and_lark(world, "showcase-journey");
    commit_line(world, "ask briar");
    commit_line(world, "travel wheel_shed");
    commit_line(world, "take chock");
    commit_line(world, "travel gate_yard");
    commit_line(world, "chock");
    let started = state_of(&pack).world_time;
    commit_line(world, "travel mill_lane");
    world.scratch = started.to_string();
    world.pack = Some(pack);
}

#[then(expr = "they arrive at the exit's cost in ticks and the arrival beat fires")]
fn they_arrive_at_the_exits_cost(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let started: u64 = world.scratch.parse().unwrap();
    let cost = lane_cost(&pack);
    let now = state_of(&pack);
    let arrived = arrival_tick(&pack);
    assert_eq!(place_of(&now, "lark"), "mill_lane");
    assert_eq!(arrived.saturating_sub(started), cost);
    assert!(cost > 0);
    assert!(is_finished(&pack));
    assert!(
        read_rounds(&pack).iter().any(|r| r["fast_forward"] == true),
        "the journey did not fast-forward"
    );
    table::wait_player_from(world, 0, "Oden and Nell are on the lane. That is the end.");
}

#[when(expr = "a player uses the consumable")]
fn a_player_uses_the_consumable(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "showcase-tonic");
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "lark");
    let (_mark, added) = table::player_send(world, "use tonic", "tonic");
    world.player_added = added;
    world.pack = Some(pack);
}

#[then(expr = "it leaves their inventory")]
fn it_leaves_their_inventory(world: &mut World) {
    let now = state_of(table::pack(world));
    assert_eq!(larks_tonic(&now), 0, "{}", world.player_added);
}

#[given(expr = "the stub GM attempts to set gold directly")]
fn the_stub_gm_attempts_to_set_gold(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "showcase-gold");
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

#[given(expr = "demo-d20 and Corin and the {word} script")]
fn demo_d20_and_corin_and_a_script(world: &mut World, script: String) {
    world.scratch = script;
}

#[when(expr = "the demo script is played with the stub GM")]
fn the_demo_script_is_played_with_the_stub_gm(world: &mut World) {
    let lines = match world.scratch.as_str() {
        "gate" => DEMO_GATE,
        "ward" => DEMO_WARD,
        "brace" => DEMO_BRACE,
        "knocked-out" => DEMO_KNOCKED_OUT,
        "goblin-bite" => DEMO_GOBLIN_BITE,
        "dusk" => DEMO_DUSK,
        other => panic!("unknown demo script: {other}"),
    };
    seat_gm_and_actor(world, "demo-d20", "corin", "demo-script");
    for line in lines {
        match line.strip_prefix("GM queues ") {
            Some(queued) => commit_gm_move(world, queued),
            None => commit_line(world, line),
        }
    }
}

#[then(expr = "the player sees only the {word} ending")]
fn the_player_sees_only_that_ending(world: &mut World, ending: String) {
    let (_, shown) = DEMO_ENDINGS
        .iter()
        .find(|(name, _)| *name == ending)
        .unwrap_or_else(|| panic!("unknown demo ending: {ending}"));
    let text = table::wait_player_from(world, 0, shown);
    for (name, other) in DEMO_ENDINGS.iter().filter(|(name, _)| *name != ending) {
        assert!(
            !text.contains(other),
            "the {name} ending was also shown\n{text}"
        );
    }
}

#[then(expr = "the player's {string} is refused with {string}")]
fn the_players_line_is_refused_with(world: &mut World, line: String, shown: String) {
    table::player_send(world, &line, &shown);
}

fn seat_gm_and_lark(world: &mut World, label: &str) -> PathBuf {
    seat_gm_and_actor(world, "sample-d20", "lark", label)
}

fn seat_gm_and_actor(world: &mut World, pack_name: &str, actor_id: &str, label: &str) -> PathBuf {
    let pack = support::copy_pack(pack_name, label);
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, actor_id);
    world.pack = Some(pack.clone());
    pack
}

fn commit_line(world: &mut World, line: &str) {
    let mark = table::player_write(world, line);
    table::wait_player_from(world, mark, "\"kind\":\"queued\"");
    let before = list_round_keys(table::pack(world)).len();
    table::gm_tool_call(world, 2, "commit", r#"{"include_round":true}"#);
    table::wait_round_count(table::pack(world), before + 1);
}

/// The GM queues `<actor> <move>` and commits it as one round.
fn commit_gm_move(world: &mut World, queued: &str) {
    let (actor_id, move_name) = queued.split_once(' ').unwrap();
    let queue = serde_json::json!({ "actor_id": actor_id, "move": move_name }).to_string();
    table::gm_tool_call(world, 3, "queue", &queue);
    let before = list_round_keys(table::pack(world)).len();
    table::gm_tool_call(world, 2, "commit", r#"{"include_round":true}"#);
    table::wait_round_count(table::pack(world), before + 1);
}

fn larks_tonic(now: &spookery_engine::State) -> u32 {
    let lark = "lark".into();
    let held = now.actors().get::<component::Item>(&lark);
    held.get("tonic").copied().unwrap_or(0)
}

fn is_finished(pack: &Path) -> bool {
    state_of(pack)
        .entities()
        .get::<component::Flag>(&entity::Id::Global)
        .contains("finished")
}

fn lane_cost(pack: &Path) -> u64 {
    let canon =
        spookery_engine::load_canon(pack).unwrap_or_else(|err| panic!("load canon: {err:?}"));
    canon
        .places
        .get("gate_yard")
        .unwrap()
        .exits
        .iter()
        .find(|exit| exit.name == "mill_lane")
        .unwrap()
        .cost
        .ticks
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

fn assert_verifies(pack: &Path) {
    let arg = pack.to_str().unwrap();
    let verified = support::run_spook(&["verify", arg]);
    let verify_out = String::from_utf8_lossy(&verified.stdout);
    let verify_err = String::from_utf8_lossy(&verified.stderr);
    assert!(
        verified.status.success(),
        "verify failed\n{verify_out}\n{verify_err}"
    );
    assert!(verify_out.contains("verified"), "{verify_out}");
}
