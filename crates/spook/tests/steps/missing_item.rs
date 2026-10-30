use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::Reason;

use crate::support::{copy_docks, insert_nell, read_round, stamp_pack_rev};
use crate::World;

use super::table;

#[given(expr = "a docks table where mark needs a knife")]
fn mark_needs_a_knife(world: &mut World) {
    let pack = copy_docks("item");
    add_knife_and_mark_rule(&pack);
    insert_nell(&pack);
    stamp_pack_rev(&pack);
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "the moves file gains a rumor row")]
fn gains_a_rumor_row(world: &mut World) {
    append_verbs(
        world,
        "\n  - id: mark\n    when: habit\n    do:\n      - rumor:\n          rumor: r_184\n          to: aldric\n          heat: 1\n",
    );
}

fn append_verbs(world: &mut World, extra: &str) {
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
        spookery_engine::first_reason(
            &spookery_engine::load_canon(table::pack(world)).unwrap_err()
        ),
        Reason::Schema
    );
}

#[then(expr = "Rowan is in docks_shop")]
fn rowan_is_in_docks_shop(world: &mut World) {
    assert_eq!(rowan_place(table::pack(world)), "docks_shop");
}

#[then(expr = "Rowan is in docks_street")]
fn rowan_is_in_docks_street(world: &mut World) {
    assert_eq!(rowan_place(table::pack(world)), "docks_street");
}

#[then(expr = "the round records the move to the street")]
fn the_round_records_the_move(world: &mut World) {
    assert_location(table::pack(world), "round-0001");
}

fn assert_location(pack: &Path, name: &str) {
    let round = read_round(pack, name);
    let hits: Vec<_> = round["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| effect["kind"] == "location" && effect["actor_id"] == "pc_rowan")
        .collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["from"], "docks_shop");
    assert_eq!(hits[0]["to"], "docks_street");
}

fn rowan_place(pack: &Path) -> String {
    table::now_of(pack)
        .actors
        .get("pc_rowan")
        .unwrap()
        .place()
        .to_string()
}

fn add_knife_and_mark_rule(pack: &Path) {
    let item = pack.join("world/items/knife.md");
    fs::create_dir_all(item.parent().unwrap()).unwrap();
    fs::write(
        &item,
        "---\nschema: item\nid: knife\ndisplay_name: Knife\ntags: []\nrules: []\n---\n\nA knife.\n",
    )
    .unwrap();
    let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    moves.push_str(
        "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    if:\n      - has_item:\n          who: self\n          id: knife\n    roll:\n      check: grit\n      target: 0\n    success:\n      - flag:\n          who: target\n          key: marked\n          value: true\n    miss:\n      - flag:\n          who: target\n          key: missed\n          value: true\n",
    );
    fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
}
