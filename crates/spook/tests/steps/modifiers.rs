//! Modifiers: stacks per application, a refresh on reapply, and one wear-off.

use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing::{edit_save, stamp_pack_rev, state_of};
use spookery_engine::{load_canon, Holder, MeterLive, ModifierLive};

use super::common::replace_in_file;
use super::triggers::{add_verb, edit_table, intent, play_round, seen_of, set_place, sit};
use crate::support::copy_docks;
use crate::World;

fn pack(world: &World) -> &Path {
    world.pack.as_deref().unwrap()
}

/// Add a row to `rules/modifiers.yaml`, writing the file first when the pack has none.
fn add_modifier(pack: &Path, id: &str, meter: &str, extra: &str) {
    let path = pack.join("rules/modifiers.yaml");
    let mut text =
        fs::read_to_string(&path).unwrap_or_else(|_| "schema: modifiers\nmodifiers:\n".into());
    text.push_str(&format!(
        "  - id: {id}\n    meter: {meter}\n    duration: 10m\n{extra}"
    ));
    fs::write(path, text).unwrap();
    stamp_pack_rev(pack);
}

/// A verb that applies each `(modifier, add)` to `who`: its target, or the actor using it.
fn add_modifier_verb(pack: &Path, verb: &str, who: &str, applies: &[(&str, i64)]) {
    let target = if who == "target" {
        "\n      target: actor"
    } else {
        ""
    };
    let effects: String = applies
        .iter()
        .map(|(id, add)| {
            format!("      - modifier:\n          who: {who}\n          id: {id}\n          add: {add}\n")
        })
        .collect();
    add_verb(
        pack,
        &format!("\n  - id: {verb}\n    when:\n      verb: {verb}{target}\n    do:\n{effects}"),
    );
    stamp_pack_rev(pack);
}

/// Play one round of these `(actor, verb)` moves, each aimed at pc_rowan.
fn hit_rowan(world: &mut World, moves: &[(&str, &str)]) {
    let pack = world.pack.clone().unwrap();
    let intents = moves
        .iter()
        .enumerate()
        .map(|(n, (actor, verb))| intent(&format!("hit-{n}"), actor, verb, Some("pc_rowan")))
        .collect();
    let played = play_round(world, &pack, intents);
    world.summary = played.changed_keys;
    world.seen = played.seen;
}

fn modifier_of(pack: &Path, id: &str) -> Option<ModifierLive> {
    state_of(pack).actors["pc_rowan"].modifiers.get(id).cloned()
}

fn stacks_of(pack: &Path, id: &str) -> Vec<i64> {
    modifier_of(pack, id)
        .map(|live| live.stacks)
        .unwrap_or_default()
}

/// How far pc_rowan's modifiers hold `meter` below its stored base and offset.
fn lowered(pack: &Path, meter: &str) -> i64 {
    let now = state_of(pack);
    let canon = load_canon(pack).unwrap_or_else(|err| panic!("load: {err:?}"));
    let rowan = Holder::Actor("pc_rowan".to_string());
    let MeterLive { base, offset } = now.actors["pc_rowan"].meters[meter].clone();
    base + offset - now.meter_value(&canon, &rowan, meter).unwrap()
}

/// Minutes from the latest round's world minute until modifier `id` wears off.
fn minutes_left(pack: &Path, id: &str) -> u64 {
    let now = state_of(pack);
    now.actors["pc_rowan"].modifiers[id].until - now.world_time
}

/// Give pc_rowan these stacks of modifier `id`, wearing off `left` minutes from now.
fn hold(pack: &Path, id: &str, stacks: Vec<i64>, left: u64) {
    let def = load_canon(pack).unwrap().modifiers[id].clone();
    edit_save(pack, |now| {
        let until = now.world_time + left;
        let live = ModifierLive {
            meter: def.meter,
            until,
            stacks,
        };
        now.actors
            .get_mut("pc_rowan")
            .unwrap()
            .modifiers
            .insert(id.to_string(), live);
    });
}

fn numbers(list: &str) -> Vec<i64> {
    list.split([',', ' '])
        .filter(|word| !word.is_empty() && *word != "and")
        .map(|word| word.parse().unwrap())
        .collect()
}

#[given(
    expr = "the docks pack with one-minute rounds and pc_rowan's grit at 20, with three PCs in the shop"
)]
fn docks_with_one_minute_rounds(world: &mut World) {
    let pack = copy_docks("modifiers");
    replace_in_file(
        &pack.join("rules/budget.yaml"),
        "live_minutes_per_round: 5\n",
        "live_minutes_per_round: 1\n",
    );
    edit_table(world, &pack, |now, seated| {
        set_place(now, "pc_wren", "docks_shop");
        set_place(now, "pc_harbormouse", "docks_shop");
        let rowan = now.actors.get_mut("pc_rowan").unwrap();
        rowan.place = "docks_shop".to_string();
        rowan.meters.get_mut("grit").unwrap().base = 20;
        sit(seated, &["pc_harbormouse", "pc_rowan", "pc_wren"]);
    });
    world.pack = Some(pack);
}

#[given(expr = "{string} modifies {word}, lasts 10 minutes, and has max_stacks {int}")]
fn modifier_with_a_cap(world: &mut World, id: String, meter: String, cap: i64) {
    add_modifier(
        pack(world),
        &id,
        &meter,
        &format!("    max_stacks: {cap}\n"),
    );
}

#[given(expr = "{string} modifies {word}, lasts 10 minutes, and has no max_stacks")]
fn modifier_without_a_cap(world: &mut World, id: String, meter: String) {
    add_modifier(pack(world), &id, &meter, "");
}

#[given(expr = "{string} is a hidden modifier of grit")]
fn hidden_modifier(world: &mut World, id: String) {
    add_modifier(pack(world), &id, "grit", "    hidden: true\n");
}

#[given(expr = "the torch verb applies burn -1 and the brand verb applies burn -2")]
fn torch_and_brand(world: &mut World) {
    add_modifier_verb(pack(world), "torch", "target", &[("burn", -1)]);
    add_modifier_verb(pack(world), "brand", "target", &[("burn", -2)]);
}

#[given(
    regex = r"^pc_rowan has \d+ stacks of (\w+) of (-?\d+(?:, -?\d+)* and -?\d+), with (\d+) minutes left$"
)]
fn rowan_holds_these_stacks(world: &mut World, id: String, list: String, left: u64) {
    hold(pack(world), &id, numbers(&list), left);
}

#[given(regex = r"^pc_rowan has (\d+) stacks of (\w+) of (-?\d+) each, with (\d+) minutes left$")]
fn rowan_holds_even_stacks(world: &mut World, count: usize, id: String, add: i64, left: u64) {
    hold(pack(world), &id, vec![add; count], left);
}

#[given(regex = r"^pc_rowan has (\d+) stacks of (\w+)$")]
fn rowan_holds_fresh_stacks(world: &mut World, count: usize, id: String) {
    hold(pack(world), &id, vec![-1; count], 10);
}

#[given(expr = "pc_rowan has 3 stacks of burn and 1 of soaked")]
fn rowan_holds_burn_and_soaked(world: &mut World) {
    hold(pack(world), "burn", vec![-1, -2, -1], 10);
    hold(pack(world), "soaked", vec![-1], 10);
}

#[given(expr = "{string} modifies sight by -1 and {string} modifies sight by -2")]
fn two_modifiers_on_sight(world: &mut World, rain: String, fog: String) {
    let pack = world.pack.clone().unwrap();
    let path = pack.join("rules/meters.yaml");
    let mut meters = fs::read_to_string(&path).unwrap();
    meters.push_str("  - id: sight\n    min: 0\n    base_cap: 10\n    max: 10\n    base: 5\n");
    fs::write(path, meters).unwrap();
    edit_save(&pack, |now| {
        for actor in now.actors.values_mut() {
            actor
                .meters
                .insert("sight".into(), MeterLive { base: 5, offset: 0 });
        }
    });
    add_modifier(&pack, &rain, "sight", "");
    add_modifier(&pack, &fog, "sight", "");
    add_modifier_verb(&pack, "weather", "self", &[(&rain, -1), (&fog, -2)]);
}

#[given(expr = "{string} modifies warmth, which is not a meter")]
fn modifier_of_no_meter(world: &mut World, id: String) {
    add_modifier(pack(world), &id, "warmth", "");
}

#[given(expr = "a verb applies {string}, which is not a modifier")]
fn verb_applies_an_undefined_modifier(world: &mut World, id: String) {
    add_modifier_verb(pack(world), "scorch", "self", &[(&id, -1)]);
}

#[when(expr = "pc_rowan is hit by the torch and then the brand in one round")]
fn rowan_is_torched_and_branded(world: &mut World) {
    hit_rowan(world, &[("pc_wren", "torch"), ("pc_harbormouse", "brand")]);
}

#[when(regex = r"^the (\w+) applies (\w+) (-?\d+)$")]
fn a_verb_applies(world: &mut World, verb: String, id: String, add: i64) {
    add_modifier_verb(pack(world), &verb, "target", &[(&id, add)]);
    hit_rowan(world, &[("pc_wren", &verb)]);
}

#[when(regex = r"^the (\w+)'s (\d+) minutes pass with no reapply$")]
fn minutes_pass(world: &mut World, id: String, minutes: u64) {
    let pack = world.pack.clone().unwrap();
    for _ in 1..minutes {
        play_round(world, &pack, Vec::new());
    }
    assert_eq!(stacks_of(&pack, &id).len(), 3, "it wore off a round early");
    world.summary = play_round(world, &pack, Vec::new()).changed_keys;
}

#[when(expr = "pc_rowan is soaked by -1 and then soaked again by -1")]
fn rowan_is_soaked_twice(world: &mut World) {
    add_modifier_verb(pack(world), "soak", "target", &[("soaked", -1)]);
    hit_rowan(world, &[("pc_wren", "soak")]);
    hit_rowan(world, &[("pc_wren", "soak")]);
}

#[when(expr = "pc_rowan stands in rain and fog")]
fn rowan_stands_in_the_weather(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    play_round(
        world,
        &pack,
        vec![intent("weather", "pc_rowan", "weather", None)],
    );
}

#[when(expr = "pc_wren burns pc_rowan and fills them with dread")]
fn wren_torments_rowan(world: &mut World) {
    add_modifier_verb(
        pack(world),
        "torment",
        "target",
        &[("burn", -1), ("dread", -1)],
    );
    hit_rowan(world, &[("pc_wren", "torment")]);
}

#[when(expr = "the pack drops burn and soaked now modifies rest")]
fn the_pack_moves_soaked(world: &mut World) {
    let text =
        "schema: modifiers\nmodifiers:\n  - id: soaked\n    meter: rest\n    duration: 10m\n";
    fs::write(pack(world).join("rules/modifiers.yaml"), text).unwrap();
}

#[when(expr = "the operator migrates the pack")]
fn the_operator_migrates(world: &mut World) {
    spookery_engine::migrate_pack(pack(world)).unwrap_or_else(|err| panic!("migrate: {err}"));
}

#[then(regex = r"^pc_rowan has (\d+) stacks? of (\w+) and (\w+) is (\d+) lower$")]
fn rowan_has_stacks_and_lower(world: &mut World, count: usize, id: String, meter: String, by: i64) {
    assert_eq!(stacks_of(pack(world), &id).len(), count);
    assert_eq!(lowered(pack(world), &meter), by);
}

#[then(regex = r"^pc_rowan has \d+ stacks of (\w+) of (-?\d+(?:, -?\d+)* and -?\d+)$")]
fn rowan_has_these_stacks(world: &mut World, id: String, list: String) {
    assert_eq!(stacks_of(pack(world), &id), numbers(&list));
}

#[then(regex = r"^pc_rowan still has (\d+) stacks of (-?\d+) and grit is (\d+) lower$")]
fn rowan_still_has_burn(world: &mut World, count: usize, add: i64, by: i64) {
    assert_eq!(stacks_of(pack(world), "burn"), vec![add; count]);
    assert_eq!(lowered(pack(world), "grit"), by);
}

#[then(regex = r"^(\w+) is (\d+) lower(?:, not \d+)?$")]
fn meter_is_lower(world: &mut World, meter: String, by: i64) {
    assert_eq!(lowered(pack(world), &meter), by);
}

#[then(regex = r"^the (\w+) ends (\d+) minutes from now, not \d+$")]
fn modifier_ends_from_now(world: &mut World, id: String, minutes: u64) {
    assert_eq!(minutes_left(pack(world), &id), minutes);
}

#[then(regex = r"^(\w+) ends (\d+) minutes after the second soaking$")]
fn modifier_ends_after_the_second(world: &mut World, id: String, minutes: u64) {
    assert_eq!(minutes_left(pack(world), &id), minutes);
}

#[then(expr = "pc_rowan has no burn and grit is back")]
fn rowan_has_no_burn(world: &mut World) {
    assert_eq!(modifier_of(pack(world), "burn"), None);
    assert_eq!(lowered(pack(world), "grit"), 0);
    let canon = load_canon(pack(world)).unwrap();
    let rowan = Holder::Actor("pc_rowan".to_string());
    assert_eq!(
        state_of(pack(world)).meter_value(&canon, &rowan, "grit"),
        Ok(20)
    );
}

#[then(expr = "pc_rowan has neither burn nor soaked")]
fn rowan_has_neither(world: &mut World) {
    assert!(state_of(pack(world)).actors["pc_rowan"]
        .modifiers
        .is_empty());
}

#[then(expr = "the GM sees {string} as a changed key")]
fn the_gm_sees_the_key(world: &mut World, key: String) {
    assert!(world.summary.contains(&key), "{:?}", world.summary);
}

#[then(expr = "pc_rowan's change line shows burn and not dread")]
fn rowan_sees_burn_only(world: &mut World) {
    let seen = seen_of(world, "pc_rowan");
    assert!(
        seen.contains(&"modifier pc_rowan burn".to_string()),
        "{seen:?}"
    );
    assert!(
        !seen.contains(&"modifier pc_rowan dread".to_string()),
        "{seen:?}"
    );
    assert!(world
        .summary
        .contains(&"modifier pc_rowan dread".to_string()));
}

#[then(expr = "pc_wren's change line shows neither")]
fn wren_sees_neither(world: &mut World) {
    let seen = seen_of(world, "pc_wren");
    assert!(
        seen.iter().all(|key| !key.starts_with("modifier ")),
        "{seen:?}"
    );
}

#[then(expr = "it refuses max_stacks 0 and the warmth meter")]
fn it_refuses_the_bad_rows(world: &mut World) {
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    for needle in [
        "rules/modifiers.yaml",
        "max_stacks below 1",
        "meter 'warmth'",
    ] {
        assert!(
            world.stderr.contains(needle),
            "missing {needle}\n{}",
            world.stderr
        );
    }
}

#[then(expr = "it refuses the scorch verb, saying scorch is not a modifier")]
fn it_refuses_the_scorch_verb(world: &mut World) {
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    let needle = "applies modifier 'scorch', which is not in rules/modifiers.yaml";
    assert!(world.stderr.contains(needle), "{}", world.stderr);
}

#[then(expr = "spook verify reproduces the journal")]
fn verify_reproduces_the_journal(world: &mut World) {
    spookery_engine::verify_pack(pack(world), None).unwrap_or_else(|err| panic!("{err:?}"));
}
