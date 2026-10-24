use std::fs;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::model::Now;
use spookery_engine::testing::journal::list_tick_keys;

use super::common::edit_save;
use super::table;
use crate::support::{self, temp_pack};
use crate::World;

#[given(expr = "a docks table with a GM and pc_rowan seated")]
fn a_docks_table_with_a_gm_and_one_pc(world: &mut World) {
    let pack = temp_pack("round-output");
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read shop: {err}"));
    let from = "  - to: docks_street\n    name: docks_street\n";
    let to = "  - to: docks_street\n    name: docks_street\n    cost:\n      time: 10m\n";
    assert!(text.contains(from), "{text}");
    fs::write(&path, text.replacen(from, to, 1)).unwrap_or_else(|err| panic!("write shop: {err}"));
    let verbs = pack.join("rules/verbs.yaml");
    let mut rules = fs::read_to_string(&verbs).unwrap_or_else(|err| panic!("read verbs: {err}"));
    rules.push_str(
        "\n  - id: hit\n    when:\n      verb: hit\n    do:\n      - meter:\n          who: self\n          key: grit\n          add: \"2d6\"\n",
    );
    fs::write(&verbs, rules).unwrap_or_else(|err| panic!("write verbs: {err}"));
    support::stamp_pack_rev(&pack);
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "pc_rowan");
    world.pack = Some(pack);
}

#[given(expr = "a docks table with pc_rowan and pc_wren seated and no GM")]
fn a_docks_table_with_two_players_and_no_gm(world: &mut World) {
    let pack = temp_pack("round-output-busy");
    edit_save(&pack, |now| {
        let until = now.world_time + 10;
        set_busy(now, "pc_rowan", until, "docks_street", "docks_street");
        set_busy(now, "pc_wren", until, "docks_shop", "docks_shop");
    });
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_rowan");
    table::seat_actor(world, &pack, "pc_wren");
    world.pack = Some(pack);
}

#[given(expr = "both players are travelling")]
fn both_players_are_travelling(world: &mut World) {
    let now = table::now_of(table::pack(world));
    for actor in ["pc_rowan", "pc_wren"] {
        assert!(
            now.actors
                .get(actor)
                .unwrap_or_else(|| panic!("no {actor}"))
                .is_busy(now.world_time),
            "{actor} is not travelling"
        );
    }
}

#[when(expr = "the GM queues {string} for {string}")]
fn the_gm_queues_a_move(world: &mut World, move_name: String, actor: String) {
    queue_move(world, &actor, &move_name, None);
}

#[when(expr = "the GM queues a 10-minute travel for {string}")]
fn the_gm_queues_a_ten_minute_travel(world: &mut World, actor: String) {
    queue_move(world, &actor, "travel", Some("docks_street"));
}

#[when(expr = "{string} sends a move")]
fn the_player_sends_a_move(world: &mut World, actor: String) {
    table::wait_host_until(world, "both seats printed", |text| {
        text.matches(STATUS_LINE).count() >= 2
    });
    let host = table::host_stdout(world).len();
    let rowan = table::actor_text(world, &actor).len();
    let wren = table::actor_text(world, "pc_wren").len();
    world.mark = rowan;
    world.host_mark = host;
    world.wren_mark = wren;
    table::actor_line(world, &actor, "wait");
}

#[then(expr = "the commit reply lists one check for {string}")]
fn the_commit_reply_lists_one_check(world: &mut World, actor: String) {
    let body = commit_body(world);
    let checks = body["checks"]
        .as_array()
        .unwrap_or_else(|| panic!("checks\n{}", world.stdout));
    assert_eq!(checks.len(), 1, "checks\n{}", world.stdout);
    let pack = table::pack(world).to_path_buf();
    let found = list_tick_keys(&pack).iter().any(|key| {
        let text = spookery_engine::testing::journal::read_record_line(&pack, key)
            .unwrap_or_else(|err| panic!("read {key}: {}", err.detail));
        text.contains(&format!("\"actor_id\":\"{actor}\"")) && text.contains("\"move\":\"grit\"")
    });
    assert!(found, "no grit intent for {actor}");
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
fn the_player_is_refused(world: &mut World, actor: String, code: String) {
    let added = table::wait_actor_from(world, &actor, world.mark, &code);
    assert!(added.contains(&format!("\"code\":\"{code}\"")), "{added}");
}

#[then(expr = "each seated player then gets a resume line and a perceive line")]
fn each_player_gets_one_refresh(world: &mut World) {
    let rowan = table::wait_actor_from(world, "pc_rowan", world.mark, "\"kind\":\"perceive\"");
    let busy = rowan
        .find("\"code\":\"BUSY\"")
        .unwrap_or_else(|| panic!("no refusal\n{rowan}"));
    let resume = rowan
        .find("\"kind\":\"resume\"")
        .unwrap_or_else(|| panic!("no resume\n{rowan}"));
    let perceive = rowan
        .find("\"kind\":\"perceive\"")
        .unwrap_or_else(|| panic!("no perceive\n{rowan}"));
    assert!(busy < resume && resume < perceive, "{rowan}");
    assert_eq!(rowan.matches("\"kind\":\"resume\"").count(), 1, "{rowan}");
    assert_eq!(rowan.matches("\"kind\":\"perceive\"").count(), 1, "{rowan}");
    let wren_mark = world.wren_mark;
    let wren = table::wait_actor_from(world, "pc_wren", wren_mark, "\"kind\":\"perceive\"");
    assert_eq!(wren.matches("\"kind\":\"resume\"").count(), 1, "{wren}");
    assert_eq!(wren.matches("\"kind\":\"perceive\"").count(), 1, "{wren}");
}

#[then(expr = "the host prints its status once")]
fn the_host_prints_its_status_once(world: &mut World) {
    let host_mark = world.host_mark;
    let text = table::wait_host_until(world, "one status", |text| {
        text.len() > host_mark && text[host_mark..].contains(STATUS_LINE)
    });
    let added = &text[host_mark..];
    assert_eq!(
        added.matches(STATUS_LINE).count(),
        1,
        "status was printed more than once\n{added}"
    );
}

const STATUS_LINE: &str = "clock shop_day 1 1 running";

fn queue_move(world: &mut World, actor: &str, move_name: &str, place: Option<&str>) {
    let target = match place {
        Some(place) => format!(r#","target":"{place}""#),
        None => String::new(),
    };
    let args = format!(r#"{{"actor_id":"{actor}","move":"{move_name}"{target}}}"#);
    let text = table::gm_call(world, 4, "intent", &args);
    assert!(
        text.contains("\"queued\""),
        "queue {move_name} for {actor}\n{text}"
    );
}

fn commit_body(world: &World) -> Value {
    let value: Value = serde_json::from_str(&world.stdout)
        .unwrap_or_else(|err| panic!("commit reply: {err}\n{}", world.stdout));
    value["result"]["structuredContent"].clone()
}

fn set_busy(now: &mut Now, actor: &str, until: u64, arrive: &str, via: &str) {
    let act = now
        .actors
        .get_mut(actor)
        .unwrap_or_else(|| panic!("no {actor}"))
        .act_mut();
    act.busy_until = Some(until);
    act.busy_for = Some(spookery_engine::model::PendingAction {
        rule: "verb:travel".to_string(),
        verb: "travel".to_string(),
        target: via.to_string(),
        arrive: Some(arrive.to_string()),
        via: Some(via.to_string()),
        source: spookery_engine::Source::Player,
    });
}
