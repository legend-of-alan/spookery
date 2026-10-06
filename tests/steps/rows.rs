use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_kernel::model::ActorLive;
use spookery_kernel::Reason;

use crate::support::temp_pack;
use crate::World;

use super::table;

#[given(expr = "a docks table where mark needs a knife")]
fn mark_needs_a_knife(world: &mut World) {
    let pack = temp_pack("item");
    add_knife_row(&pack);
    insert_nell(&pack);
    stamp(&pack);
    let port = table::free_port();
    table::listen(&pack, &format!("127.0.0.1:{port}"), port);
    table::seat_player(&pack);
    world.pack = Some(pack);
}

#[when(expr = "the moves file gains an unknown key")]
fn gains_an_unknown_key(world: &mut World) {
    let pack = table::pack(world);
    let mut moves = fs::read_to_string(pack.join("rules/moves.yaml")).unwrap();
    moves.push_str(
        "\n  - name: mark\n    audience: player\n    token: actor\n    on: named\n    item: knife\n    gear: extra\n    success:\n      - class: flag\n        key: marked\n        value: true\n",
    );
    fs::write(pack.join("rules/moves.yaml"), moves).unwrap();
}

#[when(expr = "the player marks nell")]
fn the_player_marks_nell(world: &mut World) {
    let (_mark, added) = table::player_send("mark pc_nell", "\"kind\":");
    world.player_added = added;
}

#[when(expr = "the player travels to {string}")]
fn the_player_travels_to(world: &mut World, place: String) {
    let (mark, added) = table::player_send(&format!("travel {place}"), "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the load refusal is SCHEMA")]
fn the_load_refusal_is_schema(world: &mut World) {
    assert_eq!(
        spookery_kernel::canon::load_canon(table::pack(world)).unwrap_err(),
        Reason::Schema
    );
}

#[then(expr = "Rowan is in docks_shop")]
fn rowan_is_in_docks_shop(world: &mut World) {
    assert_eq!(place_of(table::pack(world)), "docks_shop");
}

#[then(expr = "Rowan is in docks_street")]
fn rowan_is_in_docks_street(world: &mut World) {
    assert_eq!(place_of(table::pack(world)), "docks_street");
}

#[then(expr = "world_time is 1090")]
fn world_time_is_1090(world: &mut World) {
    table::wait_world_time(table::pack(world), 1090);
}

#[then(expr = "the second tick records the move to the street")]
fn the_second_tick_records_the_move(world: &mut World) {
    assert_location(table::pack(world), "tick-0002.jsonl");
}

#[then(expr = "the tick records the move to the street")]
fn the_tick_records_the_move(world: &mut World) {
    assert_location(table::pack(world), "tick-0001.jsonl");
}

fn assert_location(pack: &Path, name: &str) {
    let tick = tick(pack, name);
    let hits: Vec<_> = tick["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| effect["class"] == "location" && effect["actor_id"] == "pc_rowan")
        .collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["from"], "docks_shop");
    assert_eq!(hits[0]["to"], "docks_street");
}

fn tick(pack: &Path, name: &str) -> Value {
    let text = fs::read_to_string(pack.join("state/log").join(name))
        .unwrap_or_else(|err| panic!("read {name}: {err}"));
    serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("{err}"))
}

fn place_of(pack: &Path) -> String {
    table::now_of(pack)
        .actors
        .get("pc_rowan")
        .unwrap()
        .place()
        .to_string()
}

fn add_knife_row(pack: &Path) {
    let mut moves = fs::read_to_string(pack.join("rules/moves.yaml")).unwrap();
    moves.push_str(
        "\n  - name: mark\n    audience: player\n    token: actor\n    on: named\n    check: grit\n    item: knife\n    success:\n      - class: flag\n        key: marked\n        value: true\n    failure:\n      - class: flag\n        key: missed\n        value: true\n",
    );
    fs::write(pack.join("rules/moves.yaml"), moves).unwrap();
}

fn stamp(pack: &Path) {
    let now_path = pack.join("state/now.yaml");
    let mut now =
        spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.pack_rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    fs::write(&now_path, spookery_kernel::codec::emit_now(&now).unwrap()).unwrap();
}

fn insert_nell(pack: &Path) {
    fs::write(
        pack.join("cast/pc-nell.md"),
        "\
---
schema: cast/v0
id: pc_nell
kind: pc_template
facts: []
tick:
  eligible: false
  flags:
    - marked
    - missed
    - guarded
---
Nell keeps a guard up.
",
    )
    .unwrap();
    fs::write(
        pack.join("state/actors/pc_nell.yaml"),
        "\
schema: sheet/v0
id: pc_nell
template: cast/pc-nell.md
stats:
  grit: 100
resources: {}
inventory: []
flags: []
",
    )
    .unwrap();
    let now_path = pack.join("state/now.yaml");
    let mut now =
        spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.actors.insert(
        "pc_nell".to_string(),
        ActorLive::Pc {
            place: "docks_shop".to_string(),
            flags: Vec::new(),
            busy: false,
            known_facts: Vec::new(),
            sheet: "state/actors/pc_nell.yaml".to_string(),
        },
    );
    fs::write(&now_path, spookery_kernel::codec::emit_now(&now).unwrap()).unwrap();
}
