//! Steps for change rules that fire after each action, on the docks pack.

use std::fs;

use cucumber::{given, then, when};
use spookery_engine::{component, entity, Presence};

use super::rules::open;
use super::triggers::world_flags;
use crate::support::{append, copy_pack, edit_save, stamp_pack_rev, world_of};
use crate::World;

const PCS: [&str; 3] = ["rowan", "wren", "harbormouse"];

fn seated() -> Presence {
    Presence {
        players: 3,
        actors_present: PCS.iter().map(|id| (*id).into()).collect(),
    }
}

/// Queue each `(actor, verb)` in order, then play one round. The scratch lists the refusals
/// the round reported.
fn play_in_one_round(world: &mut World, verbs: &[(&str, &str)]) {
    let mut opened = open(world.pack.as_ref().unwrap());
    for (actor, verb) in verbs {
        if let Err(refusal) = opened.queue_player_verb(&seated(), &(*actor).into(), verb, None) {
            panic!("{actor} {verb}: {}", refusal.code.as_str());
        }
    }
    let wall = opened.world().last_round_at.clone();
    let output = opened
        .play(&seated(), &wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
    world.scratch = super::items::list_refusals(&output.reports);
}

/// Add `verbs` to `rules/verbs.yaml` and `story/<id>.yaml` holding `rules`.
fn write_rules(world: &World, verbs: &str, story: &str, rules: &str) {
    let pack = world.pack.as_ref().unwrap();
    append(&pack.join("rules/verbs.yaml"), verbs);
    fs::create_dir_all(pack.join("story")).unwrap();
    fs::write(
        pack.join(format!("story/{story}.yaml")),
        format!("schema: story\nid: {story}\nrules:\n{rules}"),
    )
    .unwrap();
    stamp_pack_rev(pack);
}

/// A verb that sets world flag `id` to `value`, when every condition in `if_` holds.
fn flag_verb(verb: &str, id: &str, value: bool, if_: &str) -> String {
    format!(
        "\n  - id: {verb}\n    when:\n      verb: {verb}\n{if_}    do:\n      - flag:\n          of: world\n          id: {id}\n          value: {value}\n"
    )
}

/// `if:` holding world flag `id` at `is`.
fn world_flag_is(id: &str, is: bool) -> String {
    format!("    if:\n      - flag:\n          of: world\n          id: {id}\n          is: {is}\n")
}

/// A change rule that sets world flag `then` when world flag `watch` becomes true and every
/// condition in `if_` holds.
fn chain_rule(id: &str, watch: &str, then: &str, if_: &str) -> String {
    format!(
        "  - id: {id}\n    when:\n      change:\n        flag: {watch}\n        becomes: true\n{if_}    do:\n      - flag:\n          of: world\n          id: {then}\n          value: true\n"
    )
}

#[given(expr = "a docks table with Rowan, Wren and Harbormouse seated in the shop")]
fn three_in_the_shop(world: &mut World) {
    let pack = copy_pack("docks", "action-triggers");
    edit_save(&pack, |now| {
        for id in PCS {
            now.actors_mut()
                .get_mut::<component::Place>(&id.into())
                .set("docks_shop")
                .unwrap();
        }
    });
    world.pack = Some(pack);
}

#[given(expr = "a rule rings the bell whenever the cellar door opens")]
fn a_bell_on_the_cellar_door(world: &mut World) {
    let verbs = flag_verb("open_cellar", "cellar_open", true, "")
        + &flag_verb("close_cellar", "cellar_open", false, "");
    let ring = "  - id: ring\n    when:\n      change:\n        flag: cellar_open\n        becomes: true\n    do:\n      - meter:\n          of: rowan\n          id: vigor\n          add: 1\n";
    write_rules(world, &verbs, "cellar", ring);
}

#[when(expr = "Rowan opens the door, Wren closes it and Harbormouse opens it in the same round")]
fn open_close_open(world: &mut World) {
    play_in_one_round(
        world,
        &[
            ("rowan", "open_cellar"),
            ("wren", "close_cellar"),
            ("harbormouse", "open_cellar"),
        ],
    );
}

#[then(expr = "the bell rings twice this round, raising Rowan's vigor by 2")]
fn the_bell_rings_twice(world: &mut World) {
    let now = world_of(world.pack.as_ref().unwrap());
    let rowan = entity::Id::Actor("rowan".into());
    assert_eq!(
        now.entities()
            .get::<component::Meter>(&rowan)
            .value("vigor"),
        Ok(3)
    );
}

#[given(expr = "a rule locks the gate when Rowan arrives in the street")]
fn a_gate_that_locks(world: &mut World) {
    let verbs = "\n  - id: go_street\n    when:\n      verb: go_street\n    do:\n      - location:\n          to: docks_street\n          teleport: true\n".to_string()
        + &flag_verb("go_gate", "gate_passed", true, &world_flag_is("gate_locked", false));
    let lock = "  - id: lock_gate\n    when:\n      change:\n        arrive: docks_street\n        subject: rowan\n    do:\n      - flag:\n          of: world\n          id: gate_locked\n          value: true\n";
    write_rules(world, &verbs, "gate", lock);
}

#[when(expr = "Rowan plays {string} and Wren plays {string} in the same round")]
fn rowan_and_wren_play(world: &mut World, rowan: String, wren: String) {
    play_in_one_round(world, &[("rowan", &rowan), ("wren", &wren)]);
}

#[then(expr = "Wren is refused because the gate is locked")]
fn wren_is_refused(world: &mut World) {
    assert_eq!(world.scratch, "wren NOT_IN_SPEC");
    let now = world_of(world.pack.as_ref().unwrap());
    assert_eq!(crate::support::place_of(&now, "rowan"), "docks_street");
    let world_flags = now.entities();
    assert!(world_flags
        .get::<component::Flag>(&entity::Id::Global)
        .contains("gate_locked"));
    assert!(!world_flags
        .get::<component::Flag>(&entity::Id::Global)
        .contains("gate_passed"));
}

#[given(
    expr = "the trigger depth is 1 and two change rules, the second's if reading the first's flag"
)]
fn two_chains_at_depth_one(world: &mut World) {
    append(
        &world.pack.as_ref().unwrap().join("rules/budget.yaml"),
        "max_trigger_depth: 1\n",
    );
    let verbs = flag_verb("pull", "pulled", true, "") + &flag_verb("push", "pushed", true, "");
    let rules = chain_rule("echo", "pushed", "echoed", &world_flag_is("rang", true))
        + &chain_rule("ring", "pulled", "rang", "");
    write_rules(world, &verbs, "chains", &rules);
}

#[then(expr = "both rules ran")]
fn both_chains_ran(world: &mut World) {
    let now = world_of(world.pack.as_ref().unwrap());
    let flags = now.entities().get::<component::Flag>(&entity::Id::Global);
    for flag in ["rang", "echoed"] {
        assert!(flags.contains(flag), "{:?}", world_flags(&now));
    }
}
