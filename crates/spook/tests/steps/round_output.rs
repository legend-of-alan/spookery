use spookery_engine::testing::{is_busy, world_of};
use std::fs;

use cucumber::{given, then, when};
use serde_json::Value;

use super::common::{edit_save, set_busy};
use super::host;
use crate::support::{self, copy_pack};
use crate::World;

#[given(expr = "a docks table with a GM and wren seated")]
async fn a_docks_table_with_a_gm_and_one_pc(world: &mut World) {
    listen_with_wren(world, false).await;
}

/// Wren is busy in Mira's rooms, so a commit fast-forwards, and Rowan's travel to the street
/// touches no seated player, so his moves keep queue order. Rowan has no player, so the GM
/// drives him.
#[given(expr = "a docks table with a GM and a travelling wren seated")]
async fn a_docks_table_with_a_gm_and_a_busy_pc(world: &mut World) {
    listen_with_wren(world, true).await;
}

async fn listen_with_wren(world: &mut World, travelling: bool) {
    let pack = copy_pack("docks", "round-output");
    let cost = "    cost:\n      ticks: 10\n";
    support::split_path(&pack, ["docks_shop", "docks_street"], [cost, ""]);
    let verbs = pack.join("rules/verbs.yaml");
    let mut rules = fs::read_to_string(&verbs).unwrap_or_else(|err| panic!("read verbs: {err}"));
    rules.push_str(
        "\n  - id: hit\n    when:\n      verb: hit\n    do:\n      - meter:\n          of: self\n          key: vigor\n          add: \"2d6\"\n",
    );
    fs::write(&verbs, rules).unwrap_or_else(|err| panic!("write verbs: {err}"));
    support::stamp_pack_rev(&pack);
    if travelling {
        edit_save(&pack, |now| {
            let until = now.world_time + 30;
            set_busy(now, "wren", until, "docks_shop", "stairs");
        });
    }
    host::listen(world, &pack).await;
    host::seat_gm(world).await;
    host::seat_actor(world, &pack, "wren").await;
    world.pack = Some(pack);
}

#[given(expr = "a docks table with rowan and wren seated and no GM")]
async fn a_docks_table_with_two_players_and_no_gm(world: &mut World) {
    let pack = copy_pack("docks", "round-output-busy");
    edit_save(&pack, |now| {
        let until = now.world_time + 10;
        set_busy(now, "rowan", until, "docks_shop", "docks_shop.docks_street");
        set_busy(
            now,
            "wren",
            until,
            "docks_street",
            "docks_shop.docks_street",
        );
    });
    host::listen(world, &pack).await;
    host::seat_actor(world, &pack, "rowan").await;
    host::seat_actor(world, &pack, "wren").await;
    world.pack = Some(pack);
}

#[given(expr = "both players are travelling")]
fn both_players_are_travelling(world: &mut World) {
    let now = world_of(host::pack(world));
    for actor in ["rowan", "wren"] {
        assert!(is_busy(&now, actor), "{actor} is not travelling");
    }
}

#[when(expr = "the GM queues {string} for {string}")]
async fn the_gm_queues_a_move(world: &mut World, move_id: String, actor: String) {
    queue_move(world, &actor, &move_id, None).await;
}

#[when(expr = "the GM queues a 10-tick travel for {string}")]
async fn the_gm_queues_a_ten_tick_travel(world: &mut World, actor: String) {
    queue_move(world, &actor, "travel", Some("docks_shop.docks_street")).await;
}

#[when(expr = "{string} sends a move")]
async fn the_player_sends_a_move(world: &mut World, actor: String) {
    host::wait_until(world, "both seats printed", |text| {
        text.matches(STATUS_LINE).count() >= 2
    })
    .await;
    let host = host::stdout(world).len();
    let rowan = host::actor_text(world, &actor).len();
    let wren = host::actor_text(world, "wren").len();
    world.mark = rowan;
    world.host_mark = host;
    world.wren_mark = wren;
    host::actor_write(world, &actor, "wait");
}

#[then(expr = "the commit reply lists one check")]
fn the_commit_reply_lists_one_check(world: &mut World) {
    let body = commit_body(world);
    let checks = body["rolls"]
        .as_array()
        .unwrap_or_else(|| panic!("checks\n{}", world.stdout));
    assert_eq!(checks.len(), 1, "checks\n{}", world.stdout);
}

#[then(expr = "the commit reply lists the committed round's dice line")]
fn the_commit_reply_lists_the_dice(world: &mut World) {
    let body = commit_body(world);
    let dice = body["dice"]
        .as_array()
        .unwrap_or_else(|| panic!("no dice\n{}", world.stdout));
    assert!(
        dice.iter()
            .any(|line| line.as_str().is_some_and(|line| line.contains("2d6 →"))),
        "dice lines did not list the committed roll\n{}",
        world.stdout
    );
}

#[then(expr = "{string} gets a refused line with code {word}")]
async fn the_player_is_refused(world: &mut World, actor: String, code: String) {
    let added = host::wait_actor_from(world, &actor, world.mark, &code).await;
    assert!(added.contains(&format!("\"code\":\"{code}\"")), "{added}");
}

#[then(expr = "each seated player then gets a resume line and a perceive line")]
async fn each_player_gets_one_refresh(world: &mut World) {
    let rowan = host::wait_actor_from(world, "rowan", world.mark, "\"kind\":\"view\"").await;
    let busy = rowan
        .find("\"code\":\"BUSY\"")
        .unwrap_or_else(|| panic!("no refusal\n{rowan}"));
    let resume = rowan
        .find("\"kind\":\"resume\"")
        .unwrap_or_else(|| panic!("no resume\n{rowan}"));
    let perceive = rowan
        .find("\"kind\":\"view\"")
        .unwrap_or_else(|| panic!("no perceive\n{rowan}"));
    assert!(busy < resume && resume < perceive, "{rowan}");
    assert_eq!(rowan.matches("\"kind\":\"resume\"").count(), 1, "{rowan}");
    assert_eq!(rowan.matches("\"kind\":\"view\"").count(), 1, "{rowan}");
    let wren_mark = world.wren_mark;
    let wren = host::wait_actor_from(world, "wren", wren_mark, "\"kind\":\"view\"").await;
    assert_eq!(wren.matches("\"kind\":\"resume\"").count(), 1, "{wren}");
    assert_eq!(wren.matches("\"kind\":\"view\"").count(), 1, "{wren}");
}

#[then(expr = "the host prints its status once")]
async fn the_host_prints_its_status_once(world: &mut World) {
    let host_mark = world.host_mark;
    let text = host::wait_until(world, "one status", |text| {
        text.len() > host_mark && text[host_mark..].contains(STATUS_LINE)
    })
    .await;
    let added = &text[host_mark..];
    assert_eq!(
        added.matches(STATUS_LINE).count(),
        1,
        "status was printed more than once\n{added}"
    );
}

const STATUS_LINE: &str = "meter mira shop_day 1 1";

async fn queue_move(world: &mut World, actor: &str, move_id: &str, place: Option<&str>) {
    let target = match place {
        Some(place) => format!(r#","target":"{place}""#),
        None => String::new(),
    };
    let args = format!(r#"{{"actor":"{actor}","verb":"{move_id}"{target}}}"#);
    let text = host::gm_tool_call(world, 4, "queue", &args).await;
    assert_eq!(
        host::read_reply(&text)["kind"],
        "queued",
        "queue {move_id} for {actor}\n{text}"
    );
}

fn commit_body(world: &World) -> Value {
    host::read_reply(&world.stdout)
}
