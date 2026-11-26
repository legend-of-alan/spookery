//! Modifiers: stacks per application, ticks added on reapply, and one expiry.

use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing::{stamp_pack_rev, world_of};
use spookery_engine::{component, entity, ActorId, Meter, Modifier};

use super::common::{add_verb, replace_in_file};
use super::meters::{assert_refused, run_check};
use super::triggers::{edit_table, intent, play_round, seen_of, set_place, set_present};
use crate::support::{self, copy_pack};
use crate::World;

fn pack(world: &World) -> &Path {
    world.pack.as_deref().unwrap()
}

/// Add a row to `rules/modifiers.yaml`, writing the file first when the pack has none.
pub(super) fn add_modifier(pack: &Path, id: &str, meter: &str, extra: &str) {
    let path = pack.join("rules/modifiers.yaml");
    let mut text =
        fs::read_to_string(&path).unwrap_or_else(|_| "schema: modifiers\nmodifiers:\n".into());
    text.push_str(&format!("  - id: {id}\n    meter: {meter}\n{extra}"));
    fs::write(path, text).unwrap();
    stamp_pack_rev(pack);
}

/// A verb that applies each `(modifier, add)` to `to` for 10 ticks: its target, or the actor using it.
fn add_modifier_verb(pack: &Path, verb: &str, to: &str, applies: &[(&str, i64)]) {
    add_timed_verb(pack, verb, to, applies, 10);
}

/// As `add_modifier_verb`, with each effect lasting `ticks`.
fn add_timed_verb(pack: &Path, verb: &str, to: &str, applies: &[(&str, i64)], ticks: u64) {
    let target = if to == "target" {
        "\n      target: actor"
    } else {
        ""
    };
    let effects: String = applies
        .iter()
        .map(|(id, add)| {
            format!("      - modifier:\n          to: {to}\n          id: {id}\n          add: {add}\n          ticks: {ticks}\n")
        })
        .collect();
    add_verb(
        pack,
        &format!("\n  - id: {verb}\n    when:\n      verb_id: {verb}{target}\n    do:\n{effects}"),
    );
    stamp_pack_rev(pack);
}

/// Play one round of these `(actor, verb)` moves, each aimed at rowan.
fn hit_rowan(world: &mut World, moves: &[(&str, &str)]) {
    let pack = world.pack.clone().unwrap();
    let intents = moves
        .iter()
        .enumerate()
        .map(|(n, (actor, verb))| intent(&format!("hit-{n}"), actor, verb, Some("rowan")))
        .collect();
    let played = play_round(world, &pack, intents);
    world.summary = played.changed_keys;
    world.seen = played.seen;
}

fn modifier_of(pack: &Path, id: &str) -> Option<Modifier> {
    let now = world_of(pack);
    let rowan = "rowan".into();
    now.actors().get::<Modifier>(&rowan).get(id).cloned()
}

fn stacks_of(pack: &Path, id: &str) -> Vec<i64> {
    modifier_of(pack, id)
        .map(|modifier| modifier.stacks)
        .unwrap_or_default()
}

/// How far rowan's modifiers hold `meter` below its stored base and offset.
fn lowered(pack: &Path, meter: &str) -> i64 {
    let now = world_of(pack);
    let rowan = entity::Id::Actor("rowan".into());
    let actor = "rowan".into();
    let meters = now.actors().get::<Meter>(&actor);
    let Meter { base, offset } = *meters.get(meter).unwrap();
    let value = now.entities().get::<component::Meter>(&rowan).value(meter);
    base + offset - value.unwrap()
}

/// Ticks from the latest round until modifier `id` expires.
fn ticks_left(pack: &Path, id: &str) -> u64 {
    let until = modifier_of(pack, id).unwrap().until;
    until.saturating_sub(world_of(pack).world_time)
}

/// Give rowan these stacks of modifier `id`, expiring `left` ticks from now, `left` being the longest so far.
fn hold(pack: &Path, id: &str, stacks: Vec<i64>, left: u64) {
    support::edit_save(pack, |now| {
        let until = now.world_time + left;
        let modifier = Modifier {
            meter: now.canon().modifiers[id].meter.clone(),
            until,
            max_ticks: left,
            stacks,
        };
        let (mut actors, rowan) = (now.actors_mut(), "rowan".into());
        actors
            .get_mut::<Modifier>(&rowan)
            .set(id, Some(modifier))
            .unwrap();
    });
}

fn numbers(list: &str) -> Vec<i64> {
    list.split([',', ' '])
        .filter(|word| !word.is_empty() && *word != "and")
        .map(|word| word.parse().unwrap())
        .collect()
}

#[given(
    expr = "the docks pack with one-tick rounds and rowan's vigor at 20, with three PCs in the shop"
)]
fn docks_with_one_tick_rounds(world: &mut World) {
    let pack = copy_pack("docks", "modifiers");
    replace_in_file(
        &pack.join("rules/budget.yaml"),
        "live_ticks_per_round: 5\n",
        "live_ticks_per_round: 1\n",
    );
    edit_table(world, &pack, |now, seated| {
        set_place(now, "wren", "docks_shop");
        set_place(now, "harbormouse", "docks_shop");
        now.actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_shop")
            .unwrap();
        support::edit_meter(now, "rowan", "vigor", |vigor| vigor.base = 20);
        set_present(seated, &["harbormouse", "rowan", "wren"]);
    });
    world.pack = Some(pack);
}

#[given(expr = "{string} modifies {word} and has max_stacks {int}")]
fn modifier_with_a_cap(world: &mut World, id: String, meter: String, cap: i64) {
    add_modifier(
        pack(world),
        &id,
        &meter,
        &format!("    max_stacks: {cap}\n"),
    );
}

#[given(expr = "{string} modifies {word} and has no max_stacks")]
fn modifier_without_a_cap(world: &mut World, id: String, meter: String) {
    add_modifier(pack(world), &id, &meter, "");
}

#[given(expr = "{string} is a hidden modifier of vigor")]
fn hidden_modifier(world: &mut World, id: String) {
    add_modifier(pack(world), &id, "vigor", "    hidden: true\n");
}

#[given(
    expr = "the jab verb applies fatigue -1 and the slam verb applies fatigue -2, each for 10 ticks"
)]
fn jab_and_slam(world: &mut World) {
    add_modifier_verb(pack(world), "jab", "target", &[("fatigue", -1)]);
    add_modifier_verb(pack(world), "slam", "target", &[("fatigue", -2)]);
}

#[given(
    regex = r"^rowan has \d+ stacks of (\w+) of (-?\d+(?:, -?\d+)* and -?\d+), with (\d+) ticks left$"
)]
fn rowan_holds_these_stacks(world: &mut World, id: String, list: String, left: u64) {
    hold(pack(world), &id, numbers(&list), left);
}

#[given(regex = r"^rowan has (\d+) stacks of (\w+) of (-?\d+) each, with (\d+) ticks left$")]
fn rowan_holds_even_stacks(world: &mut World, count: usize, id: String, add: i64, left: u64) {
    hold(pack(world), &id, vec![add; count], left);
}

#[given(regex = r"^rowan has (\d+) stacks of (\w+)$")]
fn rowan_holds_fresh_stacks(world: &mut World, count: usize, id: String) {
    hold(pack(world), &id, vec![-1; count], 10);
}

#[given(expr = "rowan has 3 stacks of fatigue and 1 of soaked")]
fn rowan_holds_fatigue_and_soaked(world: &mut World) {
    hold(pack(world), "fatigue", vec![-1, -2, -1], 10);
    hold(pack(world), "soaked", vec![-1], 10);
}

#[given(expr = "{string} modifies sight by -1 and {string} modifies sight by -2")]
fn two_modifiers_on_sight(world: &mut World, rain: String, fog: String) {
    let pack = world.pack.clone().unwrap();
    let path = pack.join("rules/meters.yaml");
    let mut meters = fs::read_to_string(&path).unwrap();
    meters.push_str("  - id: sight\n    min: 0\n    base_cap: 10\n    max: 10\n    base: 5\n");
    fs::write(path, meters).unwrap();
    support::edit_save(&pack, |now| {
        let actors: Vec<_> = now.actors().query::<ActorId>().iter().cloned().collect();
        for actor in actors {
            support::put_meter(now, &entity::Id::Actor(actor), "sight", 5);
        }
    });
    add_modifier(&pack, &rain, "sight", "");
    add_modifier(&pack, &fog, "sight", "");
    add_modifier_verb(&pack, "weather", "self", &[(&rain, -1), (&fog, -2)]);
}

#[when(expr = "rowan is hit by the jab and then the slam in one round")]
fn rowan_is_jabbed_and_slammed(world: &mut World) {
    hit_rowan(world, &[("wren", "jab"), ("harbormouse", "slam")]);
}

#[when(regex = r"^the (\w+) applies (\w+) (-?\d+) for (\d+) ticks$")]
fn a_verb_applies(world: &mut World, verb: String, id: String, add: i64, ticks: u64) {
    add_timed_verb(pack(world), &verb, "target", &[(&id, add)], ticks);
    hit_rowan(world, &[("wren", &verb)]);
}

#[when(regex = r"^the (\w+)'s (\d+) ticks pass with no reapply$")]
fn ticks_pass(world: &mut World, id: String, ticks: u64) {
    let pack = world.pack.clone().unwrap();
    for _ in 1..ticks {
        play_round(world, &pack, Vec::new());
    }
    assert_eq!(stacks_of(&pack, &id).len(), 3, "it expired a round early");
    world.summary = play_round(world, &pack, Vec::new()).changed_keys;
}

#[given(regex = r"^the (\w+) verb applies (\w+) (-?\d+) for (\d+) ticks$")]
fn verb_applies_for_ticks(world: &mut World, verb: String, id: String, add: i64, ticks: u64) {
    add_timed_verb(pack(world), &verb, "target", &[(&id, add)], ticks);
}

/// One round with the first verb, `later - 1` empty rounds, then a round with the second.
#[when(regex = r"^rowan is hit by the (\w+) and, (\d+) ticks? later, by the (\w+)$")]
fn rowan_is_hit_later(world: &mut World, first: String, later: u64, second: String) {
    let pack = world.pack.clone().unwrap();
    hit_rowan(world, &[("wren", &first)]);
    for _ in 1..later {
        play_round(world, &pack, Vec::new());
    }
    hit_rowan(world, &[("wren", &second)]);
}

#[when(expr = "rowan is soaked by -1 and then soaked again by -1")]
fn rowan_is_soaked_twice(world: &mut World) {
    add_modifier_verb(pack(world), "soak", "target", &[("soaked", -1)]);
    hit_rowan(world, &[("wren", "soak")]);
    hit_rowan(world, &[("wren", "soak")]);
}

#[when(expr = "rowan stands in rain and fog")]
fn rowan_stands_in_the_weather(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    play_round(
        world,
        &pack,
        vec![intent("weather", "rowan", "weather", None)],
    );
}

#[when(expr = "wren tires rowan and fills them with dread")]
fn wren_torments_rowan(world: &mut World) {
    add_modifier_verb(
        pack(world),
        "torment",
        "target",
        &[("fatigue", -1), ("dread", -1)],
    );
    hit_rowan(world, &[("wren", "torment")]);
}

#[when(expr = "the pack drops fatigue and soaked now modifies rest")]
fn the_pack_moves_soaked(world: &mut World) {
    let text = "schema: modifiers\nmodifiers:\n  - id: soaked\n    meter: rest\n";
    fs::write(pack(world).join("rules/modifiers.yaml"), text).unwrap();
}

const LUCK_ROW: &str = "  - id: luck
    min: 0
    base_cap: 5
    max: 5
    base: 1
";

#[given(expr = "rowan carries meter \"luck\" and the world carries meter \"tide\"")]
fn rowan_and_the_world_carry(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let rowan = pack.join("cast/pc-rowan.md");
    replace_in_file(
        &rowan,
        "meters:
",
        &format!(
            "meters:
{LUCK_ROW}"
        ),
    );
    let tide = "  - id: tide
    min: 0
    base_cap: 9
    max: 9
    base: 4
";
    let text = format!(
        "schema: meters
meters:
{tide}"
    );
    fs::write(pack.join("world/meters.yaml"), text).unwrap();
    support::edit_save(&pack, |now| {
        support::put_meter(now, &entity::Id::Actor("rowan".into()), "luck", 1);
        support::put_meter(now, &entity::Id::Global, "tide", 4);
    });
}

#[when(expr = "the pack drops luck and tide")]
fn the_pack_drops_luck_and_tide(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    replace_in_file(&pack.join("cast/pc-rowan.md"), LUCK_ROW, "");
    fs::remove_file(pack.join("world/meters.yaml")).unwrap();
}

#[when(expr = "spook check reports rowan holds {word}")]
async fn check_reports_rowan_holds(world: &mut World, id: String) {
    run_check(world).await;
    assert_refused(world, &[&format!("actor 'rowan' holds modifier '{id}'")]);
}

#[when(expr = "spook check reports the world lists meter {word}")]
async fn check_reports_the_world_lists(world: &mut World, id: String) {
    run_check(world).await;
    assert_refused(world, &[&format!("the world lists meter '{id}'")]);
}

#[when(expr = "the operator migrates the pack")]
fn the_operator_migrates(world: &mut World) {
    spookery_engine::migrate_pack(pack(world)).unwrap_or_else(|err| panic!("migrate: {err}"));
}

#[then(regex = r"^rowan has (\d+) stacks? of (\w+) and (\w+) is (\d+) lower$")]
fn rowan_has_stacks_and_lower(world: &mut World, count: usize, id: String, meter: String, by: i64) {
    assert_eq!(stacks_of(pack(world), &id).len(), count);
    assert_eq!(lowered(pack(world), &meter), by);
}

#[then(regex = r"^rowan has \d+ stacks of (\w+) of (-?\d+(?:, -?\d+)* and -?\d+)$")]
fn rowan_has_these_stacks(world: &mut World, id: String, list: String) {
    assert_eq!(stacks_of(pack(world), &id), numbers(&list));
}

#[then(regex = r"^rowan still has (\d+) stacks of (-?\d+) and vigor is (\d+) lower$")]
fn rowan_still_has_fatigue(world: &mut World, count: usize, add: i64, by: i64) {
    assert_eq!(stacks_of(pack(world), "fatigue"), vec![add; count]);
    assert_eq!(lowered(pack(world), "vigor"), by);
}

#[then(regex = r"^(\w+) is (\d+) lower(?:, not \d+)?$")]
fn meter_is_lower(world: &mut World, meter: String, by: i64) {
    assert_eq!(lowered(pack(world), &meter), by);
}

#[then(regex = r"^the (\w+) has (\d+) ticks left(?:, .*)?$")]
fn modifier_has_ticks_left(world: &mut World, id: String, ticks: u64) {
    assert_eq!(ticks_left(pack(world), &id), ticks);
}

#[then(regex = r"^(\w+) has (\d+) ticks left after the second soaking$")]
fn modifier_left_after_the_second(world: &mut World, id: String, ticks: u64) {
    assert_eq!(ticks_left(pack(world), &id), ticks);
}

#[then(expr = "rowan has no fatigue and vigor is back")]
fn rowan_has_no_fatigue(world: &mut World) {
    assert_eq!(modifier_of(pack(world), "fatigue"), None);
    assert_eq!(lowered(pack(world), "vigor"), 0);
    let rowan = entity::Id::Actor("rowan".into());
    assert_eq!(
        world_of(pack(world))
            .entities()
            .get::<component::Meter>(&rowan)
            .value("vigor"),
        Ok(20)
    );
}

#[then(expr = "rowan has neither fatigue nor soaked")]
fn rowan_has_neither(world: &mut World) {
    let now = world_of(pack(world));
    let rowan = "rowan".into();
    assert!(now.actors().get::<Modifier>(&rowan).iter().next().is_none());
}

#[then(expr = "rowan has no luck and the world has no tide")]
fn neither_keeps_the_dropped_meter(world: &mut World) {
    let now = world_of(pack(world));
    let rowan = "rowan".into();
    let meters = now.actors().get::<Meter>(&rowan);
    assert!(!meters.contains("luck"));
    assert!(meters.contains("vigor"));
    let world_meters = now.globals().query::<&Meter>();
    assert!(!world_meters.iter().any(|meters| meters.contains("tide")));
}

#[then(expr = "the GM sees {string} as a changed key")]
fn the_gm_sees_the_key(world: &mut World, key: String) {
    assert!(world.summary.contains(&key), "{:?}", world.summary);
}

#[then(expr = "rowan's change line shows fatigue and not dread")]
fn rowan_sees_fatigue_only(world: &mut World) {
    let seen = seen_of(world, "rowan");
    assert!(
        seen.contains(&"actor_modifier rowan fatigue".to_string()),
        "{seen:?}"
    );
    assert!(
        !seen.contains(&"actor_modifier rowan dread".to_string()),
        "{seen:?}"
    );
    assert!(world
        .summary
        .contains(&"actor_modifier rowan dread".to_string()));
}

#[then(expr = "wren's change line shows neither")]
fn wren_sees_neither(world: &mut World) {
    let seen = seen_of(world, "wren");
    assert!(
        seen.iter().all(|key| !key.starts_with("actor_modifier ")),
        "{seen:?}"
    );
}

#[then(expr = "spook verify reproduces the journal")]
fn verify_reproduces_the_journal(world: &mut World) {
    spookery_engine::verify_pack(pack(world), None).unwrap_or_else(|err| panic!("{err:?}"));
}

#[given(
    expr = "{string} modifies vigor, and rowan holds one stack of fatigue of -2 over a stored vigor of 5"
)]
fn fatigue_over_stored_vigor(world: &mut World, id: String) {
    add_modifier(pack(world), &id, "vigor", "    max_stacks: 4\n");
    support::edit_save(pack(world), |now| {
        support::edit_meter(now, "rowan", "vigor", |vigor| {
            *vigor = Meter { base: 5, offset: 0 }
        });
    });
    hold(pack(world), &id, vec![-2], 10);
}

/// A story whose rules watch vigor cross below 3 and below 4, each setting a flag of its own.
const STRAIN: &str = "schema: story
id: strain
rules:
  - id: under_3
    when:
      change:
        meter_id: vigor
        crosses: \"<3\"
    do:
      - flag:
          of: story:strain
          id: under_3
          value: true
  - id: under_4
    when:
      change:
        meter_id: vigor
        crosses: \"<4\"
    do:
      - flag:
          of: story:strain
          id: under_4
          value: true
";

#[when(expr = "rowan makes a move that costs 1 vigor")]
fn rowan_pays_one_vigor(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    fs::create_dir_all(pack.join("story")).unwrap();
    fs::write(pack.join("story/strain.yaml"), STRAIN).unwrap();
    add_verb(
        &pack,
        "\n  - id: brace\n    when:\n      verb_id: brace\n    cost:\n      meters:\n        vigor: 1\n    do: []\n",
    );
    stamp_pack_rev(&pack);
    let played = play_round(world, &pack, vec![intent("brace", "rowan", "brace", None)]);
    world.summary = played.changed_keys;
    let vigor = played
        .changes
        .iter()
        .filter(|(viewer, _)| viewer == "rowan")
        .flat_map(|(_, changes)| changes)
        .find(|change| change.entity_id == "rowan" && change.entry_id == "vigor");
    let after = vigor.and_then(|change| change.after.as_ref()?.as_number());
    world.scratch = after.map(|after| after.to_string()).unwrap_or_default();
}

fn strained(pack: &Path, flag: &str) -> bool {
    let strain = entity::Id::Story("strain".into());
    world_of(pack)
        .entities()
        .get::<component::Flag>(&strain)
        .contains(flag)
}

/// The change's `after` is the vigor rowan reads. Its `before` is below 4, since the rule on `<4`
/// does not fire, and the next step's rule on `<3` firing puts it at 3 or more.
#[then(expr = "the GM sees {string} as a changed key, going from 3 to 2")]
fn the_gm_sees_vigor_go_from_3_to_2(world: &mut World, key: String) {
    assert!(world.summary.contains(&key), "{:?}", world.summary);
    assert_eq!(world.scratch, "2", "rowan's reported vigor");
    assert!(!strained(pack(world), "under_4"), "vigor started below 4");
}

#[then(expr = "a change rule that waits for vigor below 3 fires")]
fn the_rule_below_three_fires(world: &mut World) {
    assert!(
        strained(pack(world), "under_3"),
        "vigor started at 3 or more"
    );
}
