use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::model::ActorLive;
use spookery_engine::MeterLive;
use spookery_engine::Reason;

use crate::support::{stamp_pack_rev, temp_pack};
use crate::World;

use super::table;

#[given(expr = "a docks table where mark needs a knife")]
fn mark_needs_a_knife(world: &mut World) {
    let pack = temp_pack("item");
    add_knife_row(&pack);
    insert_nell(&pack);
    stamp_pack_rev(&pack);
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
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
        spookery_engine::reason_of(
            spookery_engine::canon::load_canon(table::pack(world)).unwrap_err()
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
    let text = spookery_engine::testing::journal::read_record_line(pack, name)
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
        spookery_engine::store::load_save(&fs::read_to_string(&now_path).unwrap()).unwrap();
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
            act: spookery_engine::model::ActCounters::fresh(),
        },
    );
    fs::write(&now_path, spookery_engine::store::emit_save(&now).unwrap()).unwrap();
}
