//! Meters and flags that a story or a place carries.

use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing::{place_of, world_of};
use spookery_engine::{component, entity, Presence};

use super::common::{add_verb, format_meter_row};
use super::meters::migrate_and_play;
use super::triggers::{edit_table, intent, seen_of, set_place, set_present};
use crate::support::copy_pack;
use crate::World;

/// The pack this scenario already set up, or a fresh docks copy.
fn pack(world: &mut World) -> &Path {
    world
        .pack
        .get_or_insert_with(|| copy_pack("docks", "entities"))
}

/// Write `story/<id>.yaml` with these extra keys and an empty rule list to append to.
fn write_story(world: &mut World, id: &str, keys: &str) {
    let dir = pack(world).join("story");
    fs::create_dir_all(&dir).unwrap();
    let text = format!("schema: story\nid: {id}\n{keys}rules:\n");
    fs::write(dir.join(format!("{id}.yaml")), text).unwrap();
}

/// Append a change rule that runs `effect` each time rowan gains lamp oil.
fn append_story_rule(pack: &Path, story: &str, effect: &str) {
    let path = pack.join(format!("story/{story}.yaml"));
    let mut text = fs::read_to_string(&path).unwrap_or_else(|_| {
        fs::create_dir_all(pack.join("story")).unwrap();
        format!("schema: story\nid: {story}\nrules:\n")
    });
    text.push_str(&format!(
        "  - id: on_oil\n    when:\n      change:\n        item: lamp_oil\n        gained: true\n    do:\n{effect}"
    ));
    fs::write(path, text).unwrap();
}

/// Write `world/places/attic.md`, above the shop, with these extra keys.
fn write_attic(world: &mut World, keys: &str) {
    let text = format!(
        "---\nschema: place\nid: attic\nname: Attic\n{keys}---\nThe attic is over the shop.\n"
    );
    fs::write(pack(world).join("world/places/attic.md"), text).unwrap();
    let ladder = "  - path: attic -> docks_shop\n    id: ladder\n";
    crate::support::append(&pack(world).join("world/map.yaml"), ladder);
}

fn meter_effect(of: &str, key: &str) -> String {
    format!("      - meter:\n          of: {of}\n          key: {key}\n          add: 1\n")
}

fn flag_effect(of: &str, key: &str) -> String {
    format!("      - flag:\n          of: {of}\n          key: {key}\n          value: true\n")
}

/// Add verb `verb` to `rules/verbs.yaml` with these effects.
fn add_verb_doing(pack: &Path, verb: &str, effects: &str) {
    add_verb(
        pack,
        &format!("\n  - id: {verb}\n    when:\n      verb: {verb}\n    do:\n{effects}"),
    );
}

/// Put rowan in the attic, seated, and Mira in the shop.
fn stand_in_the_attic(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    edit_table(world, &pack, |now, seated| {
        set_place(now, "rowan", "attic");
        set_place(now, "mira", "docks_shop");
        set_present(seated, &["rowan"]);
    });
}

fn assert_gm_saw(world: &World, key: &str) {
    assert!(
        world.summary.iter().any(|item| item == key),
        "{key}\n{:?}",
        world.summary
    );
}

#[given(expr = "story\\/{word}.yaml declares a {string} meter")]
fn story_declares_a_meter(world: &mut World, story: String, key: String) {
    write_story(
        world,
        &story,
        &format!("meters:\n{}", format_meter_row(&key, None)),
    );
}

#[given(expr = "story\\/{word}.yaml declares {string} with hidden false")]
fn story_declares_a_shown_meter(world: &mut World, story: String, key: String) {
    let keys = format!("meters:\n{}", format_meter_row(&key, Some(false)));
    write_story(world, &story, &keys);
}

#[given(expr = "rowan in the shop and wren in the street are seated")]
fn rowan_and_wren_are_seated_apart(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    edit_table(world, &pack, |now, seated| {
        assert_eq!(place_of(now, "rowan"), "docks_shop");
        assert_eq!(place_of(now, "wren"), "docks_street");
        set_present(seated, &["rowan", "wren"]);
    });
}

#[then(expr = "rowan's and wren's change lines show dread")]
fn rowan_and_wren_see_dread(world: &mut World) {
    for id in ["rowan", "wren"] {
        let seen = seen_of(world, id);
        assert!(
            seen.contains(&"story_meter ending dread".to_string()),
            "{id}: {seen:?}"
        );
    }
}

#[then(expr = "the GM sees both light and draft")]
fn the_gm_sees_light_and_draft(world: &mut World) {
    for key in ["place_meter attic light", "place_meter attic draft"] {
        assert_gm_saw(world, key);
    }
}

#[given(expr = "a rule in story\\/{word}.yaml adds 1 to meter {word} of {string}")]
fn a_story_rule_adds(world: &mut World, story: String, key: String, of: String) {
    append_story_rule(pack(world), &story, &meter_effect(&of, &key));
}

#[given(expr = "a rule in story\\/{word}.yaml sets flag {word} of {string} to true")]
fn a_story_rule_sets(world: &mut World, story: String, key: String, of: String) {
    append_story_rule(pack(world), &story, &flag_effect(&of, &key));
}

#[given(expr = "a rule sets flag {word} of {string} to true")]
fn a_shared_rule_sets(world: &mut World, key: String, of: String) {
    let rule = format!(
        "\n  - id: on_oil\n    when:\n      change:\n        item: lamp_oil\n        gained: true\n    do:\n{}",
        flag_effect(&of, &key)
    );
    add_verb(pack(world), &rule);
}

#[when(expr = "the rule runs")]
fn the_rule_runs(world: &mut World) {
    let take = intent("take", "rowan", "take", Some("lamp_oil"));
    migrate_and_play(world, vec![take]);
}

#[then(expr = "the GM sees dread change on the ending story")]
fn the_gm_sees_dread(world: &mut World) {
    assert_gm_saw(world, "story_meter ending dread");
    let now = world_of(world.pack.as_deref().unwrap());
    let ending = "ending".into();
    let meters = now.stories().get::<component::Meter>(&ending);
    assert_eq!(meters.get("dread").unwrap().offset, 1);
}

#[then(expr = "no player's view lists dread")]
fn no_view_lists_dread(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let opened = spookery_engine::Session::peek(&pack).unwrap();
    for id in ["rowan", "wren", "harbormouse"] {
        let view = opened.perceive(&Presence::default(), id).unwrap();
        let mut listed = view.meters.iter().chain(&view.world_meters);
        assert!(listed.all(|meter| meter.id != "dread"), "{id}");
        let seen = seen_of(world, id);
        assert!(seen.iter().all(|key| !key.contains("dread")), "{seen:?}");
    }
}

#[then(expr = "a rule testing flag {word} of {string} is true sees it as set")]
fn a_rule_tests_the_flag(world: &mut World, key: String, of: String) {
    let pack = world.pack.clone().unwrap();
    add_verb(
        &pack,
        &format!(
            "\n  - id: test_mark\n    when:\n      verb: test_mark\n    if:\n      - flag:\n          of: {of}\n          key: {key}\n          is: true\n    do:\n{}",
            flag_effect("world", "mark_seen")
        ),
    );
    migrate_and_play(world, vec![intent("test", "rowan", "test_mark", None)]);
    assert!(world_of(&pack)
        .entities()
        .get::<component::Flag>(&entity::Id::Global)
        .contains("mark_seen"));
}

#[then(expr = "the GM sees the flag change on the siege story")]
fn the_gm_sees_the_siege_flag(world: &mut World) {
    assert_gm_saw(world, "story_flag siege walls_breached");
}

#[given(
    expr = "world\\/places\\/attic.md declares {string} with hidden false and {string} with no hidden key"
)]
fn attic_declares_shown_and_hidden(world: &mut World, shown: String, hidden: String) {
    let rows = format_meter_row(&shown, Some(false)) + &format_meter_row(&hidden, None);
    write_attic(world, &format!("meters:\n{rows}"));
}

#[then(expr = "the GM sees the flag change on the attic")]
fn the_gm_sees_the_attic_flag(world: &mut World) {
    assert_gm_saw(world, "place_flag attic lamp_lit");
}

#[given(expr = "rowan stands in the attic and Mira stands in the shop")]
fn rowan_in_the_attic_mira_in_the_shop(world: &mut World) {
    stand_in_the_attic(world);
}

#[given(expr = "rowan stands in the attic")]
fn rowan_in_the_attic(world: &mut World) {
    stand_in_the_attic(world);
}

#[when(expr = "a rule adds 1 to meters light and draft of {string}")]
fn a_rule_adds_light_and_draft(world: &mut World, of: String) {
    let pack = world.pack.clone().unwrap();
    let effects = meter_effect(&of, "light") + &meter_effect(&of, "draft");
    add_verb_doing(&pack, "air_out", &effects);
    edit_table(world, &pack, |_, seated| set_present(seated, &["rowan"]));
    migrate_and_play(world, vec![intent("air", "rowan", "air_out", None)]);
}

#[when(expr = "a rule sets flag {word} of {string} to true")]
fn a_verb_sets_and_runs(world: &mut World, key: String, of: String) {
    let pack = world.pack.clone().unwrap();
    add_verb_doing(&pack, "light_lamp", &flag_effect(&of, &key));
    edit_table(world, &pack, |_, seated| set_present(seated, &["rowan"]));
    migrate_and_play(world, vec![intent("light", "rowan", "light_lamp", None)]);
}

#[then(expr = "rowan's change line shows light and not draft")]
fn rowan_sees_light_not_draft(world: &mut World) {
    let seen = seen_of(world, "rowan");
    assert!(
        seen.contains(&"place_meter attic light".to_string()),
        "{seen:?}"
    );
    assert!(seen.iter().all(|key| !key.contains("draft")), "{seen:?}");
}

#[then(expr = "Mira's change line shows neither")]
fn mira_sees_neither(world: &mut World) {
    let seen = seen_of(world, "mira");
    assert!(
        seen.iter().all(|key| !key.starts_with("place_meter")),
        "{seen:?}"
    );
}

#[given(expr = "the docks pack gains world\\/places\\/attic.md")]
fn the_docks_pack_gains_the_attic(world: &mut World) {
    write_attic(world, "");
}

#[given(expr = "the docks pack gains world\\/places\\/attic.md and is migrated")]
fn the_docks_pack_gains_the_attic_and_is_migrated(world: &mut World) {
    write_attic(world, "");
    spookery_engine::migrate_pack(pack(world)).unwrap_or_else(|err| panic!("migrate: {err}"));
}

#[when(expr = "a rule adds 1 to item lamp_oil of {string}")]
fn a_rule_gives_lamp_oil(world: &mut World, of: String) {
    let effect = format!(
        "      - item:
          of: {of}
          id: lamp_oil
          add: 1
"
    );
    add_verb_doing(pack(world), "stash", &effect);
    migrate_and_play(world, vec![intent("stash", "rowan", "stash", None)]);
}

#[then(expr = "the attic holds 1 lamp_oil")]
fn the_attic_holds_lamp_oil(world: &mut World) {
    let now = world_of(world.pack.as_deref().unwrap());
    let attic = "attic".into();
    let oil = now.places().get::<component::Item>(&attic).get("lamp_oil");
    assert_eq!(oil, Some(&1));
}

#[when(expr = "a rule leaves a note in the attic")]
fn a_rule_leaves_a_note_in_the_attic(world: &mut World) {
    let effect = "      - note:
          id: note_attic
          to:
            - mira
          place: attic
          text: Mind the ladder.
";
    let pack = world.pack.clone().unwrap();
    let mut seated = Presence::default();
    set_present(&mut seated, &["rowan"]);
    world.seated.push((pack.clone(), seated));
    add_verb_doing(&pack, "scrawl", effect);
    let hatch = "  - path: docks_shop -> attic\n    id: hatch\n";
    crate::support::append(&pack.join("world/map.yaml"), hatch);
    migrate_and_play(world, vec![intent("scrawl", "mira", "scrawl", None)]);
    let climb = intent("climb", "rowan", "travel", Some("hatch"));
    migrate_and_play(world, vec![climb]);
}

#[then(expr = "rowan, standing in the attic, perceives the note")]
fn rowan_in_the_attic_perceives_the_note(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let opened =
        spookery_engine::Session::peek(&pack).unwrap_or_else(|err| panic!("peek: {err:?}"));
    let view = opened.perceive(&Presence::default(), "rowan").unwrap();
    let ids: Vec<&str> = view.notes.iter().map(|note| note.id.as_str()).collect();
    assert!(ids.contains(&"note_attic"), "{ids:?}");
}

#[then(expr = "rowan's change line does not show it")]
fn rowan_does_not_see_the_flag(world: &mut World) {
    assert_gm_saw(world, "place_flag attic lamp_lit");
    let seen = seen_of(world, "rowan");
    assert!(seen.iter().all(|key| !key.contains("lamp_lit")), "{seen:?}");
}
