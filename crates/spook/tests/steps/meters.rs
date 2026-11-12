//! Meters that one actor or the world carries.

use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::state_of;
use spookery_engine::{component, entity, load_canon, Meter, Presence};

use super::common::replace_in_file;
use super::record::read_extra_steps;
use super::table;
use super::triggers::{
    add_verb, edit_table, intent, play_round, seen_of, set_place, sit, PlayedRound,
};
use crate::support::{self, copy_docks};
use crate::World;

/// One row of a meters list: its id, its base, and its `hidden` key when it has one.
/// Every row runs from 0 to 6.
type MeterRow<'a> = (&'a str, i64, Option<bool>);

fn meter_rows(rows: &[MeterRow]) -> String {
    rows.iter()
        .map(|(id, base, hidden)| {
            let hidden = hidden.map_or(String::new(), |hidden| format!("    hidden: {hidden}\n"));
            format!(
                "  - id: {id}\n    min: 0\n    base_cap: 6\n    max: 6\n    base: {base}\n{hidden}"
            )
        })
        .collect()
}

/// Write `world/meters.yaml` with these rows.
pub(crate) fn write_world_meters(pack: &Path, rows: &[MeterRow]) {
    let text = format!("schema: meters\nmeters:\n{}", meter_rows(rows));
    fs::write(pack.join("world/meters.yaml"), text).unwrap();
}

/// Add a meter row to Mira's `meters:` list.
fn add_mira_meter(pack: &Path, row: MeterRow) {
    replace_in_file(
        &pack.join("cast/mira.md"),
        "meters:\n",
        &format!("meters:\n{}", meter_rows(&[row])),
    );
}

/// Put a habit ahead of Mira's others, so an idle round runs it first.
fn add_mira_habit(pack: &Path, id: &str, effect: &str) {
    replace_in_file(
        &pack.join("cast/mira.md"),
        "rules:\n",
        &format!("rules:\n  - id: {id}\n    when: habit\n    do:\n{effect}"),
    );
}

fn add_mira_meter_habit(pack: &Path, key: &str, add: i64) {
    let effect =
        format!("      - meter:\n          of: self\n          key: {key}\n          add: {add}\n");
    add_mira_habit(pack, &format!("bump_{key}"), &effect);
}

fn pack(world: &World) -> &Path {
    world.pack.as_deref().unwrap()
}

fn value_on(pack: &Path, id: &entity::Id, key: &str) -> Option<i64> {
    state_of(pack)
        .entity(id)
        .get::<component::Meter>()
        .value(key)
        .ok()
}

fn mira() -> entity::Id {
    entity::Id::Actor("mira".into())
}

/// Give the save the meters the pack now declares, then play one round.
pub(crate) fn migrate_and_play(world: &mut World, intents: Vec<spookery::protocol::QueueArgs>) {
    let pack = world.pack.clone().unwrap();
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    let played = play_round(world, &pack, intents);
    keep(world, played);
}

/// Add a round's change keys and lines to what the GM and each viewer have seen so far.
fn keep(world: &mut World, played: PlayedRound) {
    world.summary.extend(played.changed_keys);
    world.seen.extend(played.seen);
    world.told.extend(played.lines);
}

pub(crate) fn run_check(world: &mut World) {
    let output = support::run_spook(&["check", &support::pack_arg(pack(world))]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

pub(crate) fn assert_refused(world: &World, needles: &[&str]) {
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    for needle in needles {
        assert!(
            world.stderr.contains(needle),
            "missing {needle}\n{}",
            world.stderr
        );
    }
}

#[given(expr = "cast\\/mira.md declares a {string} meter")]
fn mira_declares_a_meter(world: &mut World, key: String) {
    let pack = world
        .pack
        .clone()
        .unwrap_or_else(|| copy_docks("cast-meter"));
    let text = fs::read_to_string(pack.join("cast/mira.md")).unwrap();
    assert!(
        text.contains(&format!("meters:\n  - id: {key}\n")),
        "{text}"
    );
    world.pack = Some(pack);
}

#[when(expr = "the pack loads")]
fn the_pack_loads(world: &mut World) {
    load_canon(pack(world)).unwrap_or_else(|err| panic!("load: {err:?}"));
}

#[then(expr = "Mira has a shop_day meter")]
fn mira_has_shop_day(world: &mut World) {
    assert_eq!(value_on(pack(world), &mira(), "shop_day"), Some(1));
}

#[then(expr = "Bram and rowan do not")]
fn bram_and_rowan_do_not(world: &mut World) {
    let now = state_of(pack(world));
    for id in ["bram", "rowan"] {
        let shop_day = (id.into(), "shop_day".to_string());
        assert!(!now.actors().meters.contains_key(&shop_day), "{id}");
    }
}

#[when(expr = "a round runs and Mira's habit reads and resets shop_day")]
fn miras_habit_reads_and_resets(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    replace_in_file(
        &pack.join("cast/mira.md"),
        "          text: Back at dusk. Oil on the shelf.\n",
        "          text: Back at dusk. Oil on the shelf.\n      - meter:\n          of: self\n          key: shop_day\n          set: 0\n",
    );
    edit_table(world, &pack, |_, _| {});
    migrate_and_play(world, Vec::new());
}

#[then(expr = "the habit runs as it did when shop_day was a clock")]
fn the_habit_runs_as_before(world: &mut World) {
    let now = state_of(pack(world));
    let note = now.notes().iter().find(|note| note.id == "note_mira_door");
    assert_eq!(note.map(|note| note.to.clone()), Some(vec!["rowan".into()]));
    assert_eq!(value_on(pack(world), &mira(), "shop_day"), Some(0));
}

#[then(expr = "the GM sees shop_day on Mira's meters")]
fn the_gm_sees_shop_day(world: &mut World) {
    let opened = spookery_engine::World::peek(pack(world)).unwrap();
    let view = opened.perceive(&Presence::default(), "mira").unwrap();
    let shop_day = view.meters.iter().find(|meter| meter.id == "shop_day");
    assert_eq!(shop_day.map(|meter| (meter.value, meter.max)), Some((0, 1)));
}

#[then(expr = "rowan's view does not list it")]
fn rowans_view_does_not_list_it(world: &mut World) {
    let opened = spookery_engine::World::peek(pack(world)).unwrap();
    let view = opened.perceive(&Presence::default(), "rowan").unwrap();
    let ids: Vec<&str> = view
        .meters
        .iter()
        .chain(&view.world_meters)
        .map(|meter| meter.id.as_str())
        .collect();
    assert!(!ids.contains(&"shop_day"), "{ids:?}");
}

#[given(expr = "Bram has a habit that adds 1 to his own shop_day")]
fn bram_adds_to_his_own_shop_day(world: &mut World) {
    replace_in_file(
        &pack(world).join("cast/bram.md"),
        "  eligible: true\n",
        "  eligible: true\nrules:\n  - id: tally_shop_day\n    when: habit\n    do:\n      - meter:\n          of: self\n          key: shop_day\n          add: 1\n",
    );
}

#[when(expr = "the author runs spook check")]
fn the_author_runs_spook_check(world: &mut World) {
    run_check(world);
}

#[then(expr = "it refuses Bram's habit, saying only Mira carries shop_day")]
fn it_refuses_brams_habit(world: &mut World) {
    assert_refused(
        world,
        &[
            "cast/bram.md",
            "rule 'tally_shop_day'",
            "only mira carries meter 'shop_day'",
        ],
    );
}

#[given(expr = "rules\\/meters.yaml defines {string}")]
fn rules_meters_defines(world: &mut World, key: String) {
    let pack = copy_docks("global-meter");
    let text = fs::read_to_string(pack.join("rules/meters.yaml")).unwrap();
    assert!(text.contains(&format!("  - id: {key}\n")), "{text}");
    world.pack = Some(pack);
}

#[given(expr = "cast\\/mira.md also declares a {string} meter")]
fn mira_also_declares(world: &mut World, key: String) {
    add_mira_meter(pack(world), (&key, 1, None));
}

#[then(expr = "it refuses the second grit")]
fn it_refuses_the_second_grit(world: &mut World) {
    assert_refused(
        world,
        &[
            "cast/mira.md",
            "meter 'grit' is already in rules/meters.yaml",
        ],
    );
}

#[given(expr = "rowan and Mira stand in the shop")]
fn rowan_and_mira_stand_in_the_shop(world: &mut World) {
    let pack = copy_docks("meter-sight");
    add_mira_meter_habit(&pack, "shop_day", 1);
    edit_table(world, &pack, |now, seated| {
        set_place(now, "mira", "docks_shop");
        support::edit_meter(now, "mira", "shop_day", |shop_day| shop_day.base = 0);
        sit(seated, &["rowan"]);
    });
    world.pack = Some(pack);
}

#[when(expr = "Mira's habit adds 1 to her shop_day")]
fn miras_habit_adds_to_shop_day(world: &mut World) {
    let queued = intent("gm-bump", "mira", "bump_shop_day", None);
    migrate_and_play(world, vec![queued]);
    assert_eq!(value_on(pack(world), &mira(), "shop_day"), Some(1));
}

#[then(expr = "Mira's change line shows shop_day")]
fn miras_change_line_shows_shop_day(world: &mut World) {
    let seen = seen_of(world, "mira");
    assert!(
        seen.contains(&"actor_meter mira shop_day".to_string()),
        "{seen:?}"
    );
}

#[then(expr = "rowan's change line does not")]
fn rowans_change_line_does_not(world: &mut World) {
    let seen = seen_of(world, "rowan");
    assert!(seen.iter().all(|key| !key.contains("shop_day")), "{seen:?}");
}

#[given(expr = "rowan stands in the shop")]
fn rowan_stands_in_the_shop(world: &mut World) {
    let pack = copy_docks("flag-sight");
    replace_in_file(
        &pack.join("cast/mira.md"),
        "  eligible: true\n",
        "  eligible: true\nflags:\n  - shop_open\n",
    );
    add_verb(
        &pack,
        "\n  - id: ring_bells\n    when:\n      verb: ring_bells\n    do:\n      - flag:\n          of: world\n          key: bells_rung\n          value: true\n  - id: open_shop\n    when:\n      verb: open_shop\n    do:\n      - flag:\n          of: mira\n          key: shop_open\n          value: true\n",
    );
    edit_table(world, &pack, |now, seated| {
        set_place(now, "mira", "docks_shop");
        sit(seated, &["rowan"]);
    });
    world.pack = Some(pack);
}

#[when(expr = "a rule sets the world flag {string}")]
fn a_rule_sets_the_world_flag(world: &mut World, flag: String) {
    assert_eq!(flag, "bells_rung");
    migrate_and_play(world, vec![intent("ring", "rowan", "ring_bells", None)]);
}

#[when(expr = "a rule sets Mira's flag {string}")]
fn a_rule_sets_miras_flag(world: &mut World, flag: String) {
    assert_eq!(flag, "shop_open");
    migrate_and_play(world, vec![intent("open", "rowan", "open_shop", None)]);
}

#[then(expr = "rowan's change line shows neither flag")]
fn rowans_change_line_shows_neither_flag(world: &mut World) {
    let seen = seen_of(world, "rowan");
    assert!(seen.iter().all(|key| !key.contains("flag")), "{seen:?}");
}

#[then(expr = "the GM sees both as changed keys")]
fn the_gm_sees_both_flags(world: &mut World) {
    for key in ["world_flag bells_rung", "actor_flag mira shop_open"] {
        assert!(
            world.summary.iter().any(|item| item == key),
            "{key}\n{:?}",
            world.summary
        );
    }
}

#[given(expr = "world\\/meters.yaml declares a {string} meter at {int}")]
fn world_meters_declares_at(world: &mut World, key: String, base: i64) {
    let pack = copy_docks("world-meter");
    write_world_meters(&pack, &[(&key, base, None)]);
    world.pack = Some(pack);
}

#[given(expr = "world\\/meters.yaml declares a {string} meter")]
fn world_meters_declares(world: &mut World, key: String) {
    world_meters_declares_at(world, key, 6);
}

/// A story change rule that adds -1 to the world's dawn each time rowan gains lamp oil.
fn write_dawn_story(pack: &Path, story: &str, more: &str) {
    fs::create_dir_all(pack.join("story")).unwrap();
    let text = format!(
        "schema: story\nid: {story}\nrules:\n  - id: dawn_nears\n    when:\n      change:\n        item: lamp_oil\n        gained: true\n    do:\n      - meter:\n          of: world\n          key: dawn\n          add: -1\n{more}"
    );
    fs::write(pack.join(format!("story/{story}.yaml")), text).unwrap();
}

#[given(expr = "story\\/siege.yaml has a rule that adds -1 to meter dawn of {string}")]
fn siege_counts_down(world: &mut World, of: String) {
    assert_eq!(of, "world");
    write_dawn_story(pack(world), "siege", "");
}

#[given(expr = "story\\/vigil.yaml has a rule that adds -1 to meter dawn of {string}")]
fn vigil_counts_down(world: &mut World, of: String) {
    assert_eq!(of, "world");
    write_dawn_story(pack(world), "vigil", "");
}

#[given(expr = "story\\/vigil.yaml has a change rule that watches dawn cross {string}")]
fn vigil_watches_dawn(world: &mut World, crosses: String) {
    let watch = format!(
        "  - id: dawn_breaks\n    when:\n      change:\n        meter: dawn\n        crosses: \"{crosses}\"\n    do:\n      - flag:\n          of: world\n          key: dawn_broke\n          value: true\n"
    );
    write_dawn_story(pack(world), "vigil", &watch);
}

#[when(expr = "both rules run in one round, three rounds in a row")]
fn both_rules_run_three_rounds(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    for round in 1..=3 {
        let take = intent(&format!("take-{round}"), "rowan", "take", Some("lamp_oil"));
        let played = play_round(world, &pack, vec![take]);
        keep(world, played);
    }
}

#[then(expr = "dawn is 0, held once by the world")]
fn dawn_is_zero(world: &mut World) {
    assert_eq!(value_on(pack(world), &entity::Id::Global, "dawn"), Some(0));
}

#[then(expr = "no actor carries a dawn meter")]
fn no_actor_carries_dawn(world: &mut World) {
    let now = state_of(pack(world));
    for id in now.actors().control_type.keys() {
        let dawn = (id.clone(), "dawn".to_string());
        assert!(!now.actors().meters.contains_key(&dawn), "{id}");
    }
}

#[then(expr = "the vigil's change rule fires once")]
fn the_vigil_fires_once(world: &mut World) {
    let pack = pack(world);
    let fired = spookery_engine::testing::journal::list_round_keys(pack)
        .iter()
        .map(|name| support::read_round(pack, name))
        .flat_map(|round| read_extra_steps(&round))
        .filter(|name| name == "story:vigil/dawn_breaks")
        .count();
    assert_eq!(fired, 1);
    assert!(state_of(pack)
        .entity(&entity::Id::Global)
        .get::<component::Flag>()
        .contains("dawn_broke"));
}

/// Give the world a `morning` meter from 0, and append effects to the vigil's dawn rule, the
/// last rule in its file.
fn add_to_dawn_rule(pack: &Path, effects: &str) {
    let meters = pack.join("world/meters.yaml");
    let mut text = fs::read_to_string(&meters).unwrap();
    text.push_str(&meter_rows(&[("morning", 0, None)]));
    fs::write(meters, text).unwrap();
    let story = pack.join("story/vigil.yaml");
    let mut text = fs::read_to_string(&story).unwrap();
    text.push_str(effects);
    fs::write(story, text).unwrap();
}

fn add_morning(of: &str) -> String {
    format!(
        "      - meter:
          of: {of}
          key: morning
          add: 1
"
    )
}

#[given(expr = "that rule narrates {string} and adds 1 to meter morning of {string}")]
fn the_dawn_rule_narrates(world: &mut World, text: String, of: String) {
    let narrate = format!(
        "      - narrate:
          text: {text}
"
    );
    add_to_dawn_rule(pack(world), &(narrate + &add_morning(&of)));
}

#[given(
    expr = "story\\/vigil.yaml has a change rule that watches dawn cross {string} and adds 1 to meter morning of {string}"
)]
fn vigil_watches_dawn_and_adds_morning(world: &mut World, crosses: String, of: String) {
    vigil_watches_dawn(world, crosses);
    add_to_dawn_rule(pack(world), &add_morning(&of));
}

#[given(expr = "nobody is seated")]
fn nobody_is_seated(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    edit_table(world, &pack, |_, _| {});
}

#[when(expr = "a rule adds -1 to meter dawn of {string}")]
fn a_rule_adds_to_dawn(world: &mut World, of: String) {
    let pack = world.pack.clone().unwrap();
    add_verb(
        &pack,
        &format!(
            "
  - id: dawn_falls
    when:
      verb: dawn_falls
    do:
      - meter:
          of: {of}
          key: dawn
          add: -1
"
        ),
    );
    migrate_and_play(world, vec![intent("dawn", "rowan", "dawn_falls", None)]);
}

#[when(expr = "a habit adds -1 to meter dawn of {string}")]
fn a_habit_adds_to_dawn(world: &mut World, of: String) {
    let effect = format!(
        "      - meter:
          of: {of}
          key: dawn
          add: -1
"
    );
    add_mira_habit(pack(world), "dawn_falls", &effect);
    migrate_and_play(world, Vec::new());
}

#[then(expr = "rowan and nell each see {string} once")]
fn rowan_and_nell_each_see_once(world: &mut World, text: String) {
    for viewer in ["rowan", "nell"] {
        let seen = world
            .told
            .iter()
            .filter(|(to, line)| to == viewer && line == &text)
            .count();
        assert_eq!(seen, 1, "{viewer}: {:?}", world.told);
    }
}

#[given(expr = "Mira carries no dawn meter")]
fn mira_carries_no_dawn(world: &mut World) {
    let canon = load_canon(pack(world)).unwrap_or_else(|err| panic!("load: {err:?}"));
    assert!(canon.meter_def(mira().as_ref(), "dawn").is_none());
}

#[given(expr = "Mira has a habit that adds 1 to meter dawn of {string}")]
fn mira_adds_to_dawn(world: &mut World, of: String) {
    assert_eq!(of, "self");
    add_mira_meter_habit(pack(world), "dawn", 1);
}

#[then(expr = "it refuses Mira's habit, saying Mira carries no dawn meter")]
fn it_refuses_miras_habit(world: &mut World) {
    assert_refused(
        world,
        &[
            "cast/mira.md",
            "rule 'bump_dawn'",
            "mira carries no meter 'dawn'",
        ],
    );
}

#[given(expr = "cast\\/mira.md declares her own {string} meter at {int}")]
fn mira_declares_her_own(world: &mut World, key: String, base: i64) {
    add_mira_meter(pack(world), (&key, base, None));
}

#[when(expr = "one round runs")]
fn one_round_runs(world: &mut World) {
    migrate_and_play(world, Vec::new());
}

#[then(expr = "Mira's dawn is {int}")]
fn miras_dawn_is(world: &mut World, value: i64) {
    assert_eq!(value_on(pack(world), &mira(), "dawn"), Some(value));
}

#[then(expr = "the world's dawn is still {int}")]
fn the_worlds_dawn_is_still(world: &mut World, value: i64) {
    the_worlds_meter_is(world, "dawn".to_string(), value);
}

#[then(expr = "the world's {word} is {int}")]
fn the_worlds_meter_is(world: &mut World, key: String, value: i64) {
    assert_eq!(
        value_on(pack(world), &entity::Id::Global, &key),
        Some(value)
    );
}

#[then(expr = "it refuses the rule, saying a world change rule has no self")]
fn it_refuses_the_world_rule(world: &mut World) {
    assert_refused(
        world,
        &[
            "story/tide.yaml",
            "rule 'mark' watches a world change, which has no self",
        ],
    );
}

#[given(
    expr = "world\\/meters.yaml declares {string} with hidden false and {string} with no hidden key"
)]
fn world_meters_shown_and_hidden(world: &mut World, shown: String, hidden: String) {
    let pack = copy_docks("world-sight");
    write_world_meters(&pack, &[(&shown, 6, Some(false)), (&hidden, 6, None)]);
    world.pack = Some(pack);
}

#[when(expr = "a rule adds -1 to meter dawn of {string} and to meter doom of {string}")]
fn a_rule_darkens_dawn_and_doom(world: &mut World, first: String, second: String) {
    assert_eq!((first.as_str(), second.as_str()), ("world", "world"));
    let pack = world.pack.clone().unwrap();
    add_verb(
        &pack,
        "\n  - id: darken\n    when:\n      verb: darken\n    do:\n      - meter:\n          of: world\n          key: dawn\n          add: -1\n      - meter:\n          of: world\n          key: doom\n          add: -1\n",
    );
    edit_table(world, &pack, |_, seated| sit(seated, &["rowan"]));
    migrate_and_play(world, vec![intent("darken", "rowan", "darken", None)]);
}

#[then(expr = "rowan's change line shows dawn and not doom")]
fn rowan_sees_dawn_not_doom(world: &mut World) {
    let seen = seen_of(world, "rowan");
    assert!(seen.contains(&"world_meter dawn".to_string()), "{seen:?}");
    assert!(!seen.contains(&"world_meter doom".to_string()), "{seen:?}");
}

#[then(expr = "the GM sees both")]
fn the_gm_sees_both_meters(world: &mut World) {
    for key in ["world_meter dawn", "world_meter doom"] {
        assert!(
            world.summary.iter().any(|item| item == key),
            "{key}\n{:?}",
            world.summary
        );
    }
}

#[given(expr = "dawn has fallen to {int} while rowan's player was away")]
fn dawn_has_fallen(world: &mut World, value: i64) {
    let pack = world.pack.clone().unwrap();
    edit_table(world, &pack, |now, _| {
        let fallen = Meter {
            base: 6,
            offset: value - 6,
        };
        let world_meters = now
            .entity_mut(&entity::Id::Global)
            .get_mut::<component::Meter>()
            .set("dawn", Some(fallen));
        world_meters.unwrap();
        support::put_meter(now, &entity::Id::Global, "doom", 6);
    });
    assert_eq!(value_on(&pack, &entity::Id::Global, "dawn"), Some(value));
}

#[when(expr = "the player reconnects and claims rowan")]
fn the_player_reconnects(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "rowan");
}

/// The `world_meters` of the view rowan's seat got after its claim.
fn catch_up_meters(world: &World) -> Vec<(String, i64)> {
    let text = table::actor_text(world, "rowan");
    let view = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|message| message["body"].clone())
        .find(|body| body["kind"] == "view")
        .unwrap_or_else(|| panic!("no view\n{text}"));
    let meters = view["world_meters"].as_array().cloned().unwrap_or_default();
    meters
        .iter()
        .map(|meter| {
            (
                meter["id"].as_str().unwrap().to_string(),
                meter["value"].as_i64().unwrap(),
            )
        })
        .collect()
}

#[then(expr = "the catch-up view lists dawn at {int}")]
fn the_catch_up_view_lists_dawn(world: &mut World, value: i64) {
    let meters = catch_up_meters(world);
    assert!(meters.contains(&("dawn".to_string(), value)), "{meters:?}");
}

#[then(expr = "it does not list doom")]
fn it_does_not_list_doom(world: &mut World) {
    let meters = catch_up_meters(world);
    assert!(meters.iter().all(|(id, _)| id != "doom"), "{meters:?}");
}

#[given(expr = "a verb narrates {string} with no to")]
fn a_verb_narrates_with_no_to(world: &mut World, text: String) {
    let pack = copy_docks("narrate-no-to");
    add_verb(
        &pack,
        &format!("\n  - id: ring\n    when:\n      verb: ring\n    do:\n      - narrate:\n          text: {text}\n"),
    );
    world.pack = Some(pack);
}

#[then(expr = "it refuses the verb, saying its narrate has no to")]
fn it_refuses_the_verb_narrate(world: &mut World) {
    assert_refused(
        world,
        &["rules/verbs.yaml", "rule 'ring' has a narrate with no to"],
    );
}
