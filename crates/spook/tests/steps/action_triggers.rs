//! Steps for change rules that fire after each action, on the docks pack.

use std::fs;

use cucumber::{given, then, when};
use spookery_engine::Presence;

use super::record::read_extra_steps;
use super::rules::open;
use crate::support::{append, copy_docks, edit_save, read_round, stamp_pack_rev, state_of};
use crate::World;

const PCS: [&str; 3] = ["pc_rowan", "pc_wren", "pc_harbormouse"];

fn seated() -> Presence {
    Presence {
        players: 3,
        actors_present: PCS.iter().map(|id| id.to_string()).collect(),
    }
}

/// Queue each `(actor, verb)` in order, then play one round.
fn play_in_one_round(world: &World, moves: &[(&str, &str)]) {
    let mut opened = open(world.pack.as_ref().unwrap());
    for (actor, verb) in moves {
        if let Err(refusal) = opened.queue_player_move(&seated(), actor, verb, None) {
            panic!("{actor} {verb}: {}", refusal.code.as_str());
        }
    }
    let wall = opened.state().last_round_at.clone();
    opened
        .play(&seated(), &wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
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

/// A verb that sets world flag `key` to `value`, when every condition in `if_` holds.
fn flag_verb(verb: &str, key: &str, value: bool, if_: &str) -> String {
    format!(
        "\n  - id: {verb}\n    when:\n      verb: {verb}\n{if_}    do:\n      - flag:\n          who: world\n          key: {key}\n          value: {value}\n"
    )
}

/// `if:` holding world flag `key` at `is`.
fn world_flag_is(key: &str, is: bool) -> String {
    format!(
        "    if:\n      - flag:\n          who: world\n          key: {key}\n          is: {is}\n"
    )
}

/// A change rule that sets world flag `then` when world flag `watch` becomes true and every
/// condition in `if_` holds.
fn chain_rule(id: &str, watch: &str, then: &str, if_: &str) -> String {
    format!(
        "  - id: {id}\n    when:\n      change:\n        flag: {watch}\n        becomes: true\n{if_}    do:\n      - flag:\n          who: world\n          key: {then}\n          value: true\n"
    )
}

fn latest_round(world: &World) -> serde_json::Value {
    let pack = world.pack.as_ref().unwrap();
    let round = state_of(pack).round_index;
    read_round(pack, &format!("round-{round:04}"))
}

#[given(expr = "a docks table with Rowan, Wren and Harbormouse seated in the shop")]
fn three_in_the_shop(world: &mut World) {
    let pack = copy_docks("action-triggers");
    edit_save(&pack, |now| {
        for id in PCS {
            now.actors.get_mut(id).unwrap().place = "docks_shop".to_string();
        }
    });
    world.pack = Some(pack);
}

#[given(expr = "a rule rings the bell whenever the cellar door opens")]
fn a_bell_on_the_cellar_door(world: &mut World) {
    let verbs = flag_verb("open_cellar", "cellar_open", true, "")
        + &flag_verb("close_cellar", "cellar_open", false, "");
    let ring = "  - id: ring\n    when:\n      change:\n        flag: cellar_open\n        becomes: true\n    do: []\n";
    write_rules(world, &verbs, "cellar", ring);
}

#[when(expr = "Rowan opens the door, Wren closes it and Harbormouse opens it in the same round")]
fn open_close_open(world: &mut World) {
    play_in_one_round(
        world,
        &[
            ("pc_rowan", "open_cellar"),
            ("pc_wren", "close_cellar"),
            ("pc_harbormouse", "open_cellar"),
        ],
    );
}

#[then(expr = "the bell rings twice this round")]
fn the_bell_rings_twice(world: &mut World) {
    let round = latest_round(world);
    let rings = read_extra_steps(&round)
        .iter()
        .filter(|name| name.as_str() == "story:cellar/ring")
        .count();
    assert_eq!(rings, 2, "{round}");
}

#[given(expr = "a rule locks the gate when Rowan arrives in the street")]
fn a_gate_that_locks(world: &mut World) {
    let verbs = "\n  - id: go_street\n    when:\n      verb: go_street\n    do:\n      - location:\n          to: docks_street\n          teleport: true\n".to_string()
        + &flag_verb("go_gate", "gate_passed", true, &world_flag_is("gate_locked", false));
    let lock = "  - id: lock_gate\n    when:\n      change:\n        arrive: docks_street\n        who: pc_rowan\n    do:\n      - flag:\n          who: world\n          key: gate_locked\n          value: true\n";
    write_rules(world, &verbs, "gate", lock);
}

#[when(expr = "Rowan plays {string} and Wren plays {string} in the same round")]
fn rowan_and_wren_play(world: &mut World, rowan: String, wren: String) {
    play_in_one_round(world, &[("pc_rowan", &rowan), ("pc_wren", &wren)]);
}

#[then(expr = "Wren is refused because the gate is locked")]
fn wren_is_refused(world: &mut World) {
    let round = latest_round(world);
    let refused = round["refused"].as_array().unwrap();
    assert_eq!(refused.len(), 1, "{round}");
    assert_eq!(refused[0]["code"], "NOT_IN_SPEC", "{round}");
    let now = state_of(world.pack.as_ref().unwrap());
    assert_eq!(now.actors["pc_rowan"].place, "docks_street");
    assert!(now.flags.iter().any(|flag| flag == "gate_locked"));
    assert!(!now.flags.iter().any(|flag| flag == "gate_passed"));
}

#[given(
    expr = "the trigger depth is 1 and two change rules, the second's if reading the first's flag"
)]
fn two_chains_at_depth_one(world: &mut World) {
    append(
        &world.pack.as_ref().unwrap().join("rules/budget.yaml"),
        "max_trigger_depth: 1\n",
    );
    // `echo` comes first, so a pass that saw both pulls at once would read `rang` unset.
    let verbs = flag_verb("pull", "pulled", true, "") + &flag_verb("push", "pushed", true, "");
    let rules = chain_rule("echo", "pushed", "echoed", &world_flag_is("rang", true))
        + &chain_rule("ring", "pulled", "rang", "");
    write_rules(world, &verbs, "chains", &rules);
}

#[then(expr = "both rules ran and the round logged no trigger_depth")]
fn both_chains_ran(world: &mut World) {
    let round = latest_round(world);
    assert_eq!(round["refused"], serde_json::json!([]), "{round}");
    let steps = read_extra_steps(&round);
    assert!(!steps.iter().any(|name| name == "trigger_depth"), "{round}");
    let now = state_of(world.pack.as_ref().unwrap());
    for flag in ["rang", "echoed"] {
        assert!(now.flags.iter().any(|held| held == flag), "{:?}", now.flags);
    }
}
