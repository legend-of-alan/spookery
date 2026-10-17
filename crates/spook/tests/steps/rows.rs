use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_kernel::api::meter::MeterLive;
use spookery_kernel::api::model::ActorLive;
use spookery_kernel::api::Reason;

use crate::support::temp_pack;
use crate::World;

use super::table;

#[given(expr = "a docks table where mark needs a knife")]
fn mark_needs_a_knife(world: &mut World) {
    let pack = temp_pack("item");
    add_knife_row(&pack);
    insert_nell(&pack);
    stamp(&pack);
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "the moves file gains an unknown key")]
fn gains_an_unknown_key(world: &mut World) {
    append_moves(
        world,
        "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    gear: extra\n    do: []\n",
    );
}

#[when(expr = "the moves file gains a system audience")]
fn gains_a_system_audience(world: &mut World) {
    append_moves(
        world,
        "\n  - id: night\n    when: habit\n    audience: system\n    do: []\n",
    );
}

#[when(expr = "the moves file gains a relationship row")]
fn gains_a_relationship_row(world: &mut World) {
    append_moves(
        world,
        "\n  - id: mark\n    when: habit\n    do:\n      - relationship:\n          to: bram\n          delta: -1\n",
    );
}

#[when(expr = "the moves file gains a reputation row")]
fn gains_a_reputation_row(world: &mut World) {
    append_moves(
        world,
        "\n  - id: mark\n    when: habit\n    do:\n      - reputation:\n          faction: harbor\n          delta: 1\n",
    );
}

#[when(expr = "the moves file gains a rumor row")]
fn gains_a_rumor_row(world: &mut World) {
    append_moves(
        world,
        "\n  - id: mark\n    when: habit\n    do:\n      - rumor:\n          rumor: r_184\n          to: aldric\n          heat: 1\n",
    );
}

fn append_moves(world: &mut World, extra: &str) {
    let pack = table::pack(world);
    let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    moves.push_str(extra);
    fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
}

#[when(expr = "the player marks nell")]
fn the_player_marks_nell(world: &mut World) {
    let (_mark, added) = table::player_send(world, "mark pc_nell", "\"kind\":");
    world.player_added = added;
}

#[when(expr = "the player travels to {string}")]
fn the_player_travels_to(world: &mut World, place: String) {
    let (mark, added) = table::player_send(world, &format!("travel {place}"), "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the load refusal is SCHEMA")]
fn the_load_refusal_is_schema(world: &mut World) {
    assert_eq!(
        spookery_kernel::api::reason_of(
            spookery_kernel::api::canon::load_canon(table::pack(world)).unwrap_err()
        ),
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
    assert_location(table::pack(world), "tick-0002");
}

#[then(expr = "the tick records the move to the street")]
fn the_tick_records_the_move(world: &mut World) {
    assert_location(table::pack(world), "tick-0001");
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
    let text = spookery_kernel::api::journal::read_record_line(pack, name)
        .unwrap_or_else(|err| panic!("read {name}: {}", err.detail));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{err}"))
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
    let item = pack.join("world/items/knife.md");
    fs::create_dir_all(item.parent().unwrap()).unwrap();
    fs::write(
        &item,
        "---\nschema: item\nid: knife\nname: Knife\ntags: []\nrules: []\n---\n\nA knife.\n",
    )
    .unwrap();
    let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    moves.push_str(
        "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    if:\n      - has_item:\n          who: self\n          id: knife\n    roll:\n      check: grit\n      target: 0\n    success:\n      - flag:\n          who: target\n          key: marked\n          value: true\n    failure:\n      - flag:\n          who: target\n          key: missed\n          value: true\n",
    );
    fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
}

fn stamp(pack: &Path) {
    let now_path = pack.join("state/save.yaml");
    let mut now =
        spookery_kernel::api::store::codec::load_save(&fs::read_to_string(&now_path).unwrap())
            .unwrap();
    now.pack_rev = spookery_kernel::api::canon::pack_rev(pack).unwrap();
    fs::write(
        &now_path,
        spookery_kernel::api::store::codec::emit_save(&now).unwrap(),
    )
    .unwrap();
}

fn insert_nell(pack: &Path) {
    fs::write(
        pack.join("cast/pc-nell.md"),
        "\
---
schema: cast
id: pc_nell
kind: pc_template
display_name: Nell
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
    let now_path = pack.join("state/save.yaml");
    let mut now =
        spookery_kernel::api::store::codec::load_save(&fs::read_to_string(&now_path).unwrap())
            .unwrap();
    now.actors.insert(
        "pc_nell".to_string(),
        ActorLive::Pc {
            place: "docks_shop".to_string(),
            flags: Vec::new(),

            meters: BTreeMap::from([
                (
                    "grit".to_string(),
                    MeterLive {
                        base: 100,
                        offset: 0,
                    },
                ),
                ("rest".to_string(), MeterLive { base: 3, offset: 0 }),
            ]),
            inventory: BTreeMap::new(),
            wielded: None,
            known_facts: Vec::new(),
            visited: vec!["docks_shop".to_string()],
            act: spookery_kernel::api::model::ActCounters::fresh(),
        },
    );
    fs::write(
        &now_path,
        spookery_kernel::api::store::codec::emit_save(&now).unwrap(),
    )
    .unwrap();
}
