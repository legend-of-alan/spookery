//! Meters and flags that a story or a place carries.

use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing::state_of;
use spookery_engine::{Presence, World as Opened};

use super::meters::{assert_refused, migrate_and_play};
use super::triggers::{add_verb, edit_table, intent, seen_of, set_place, sit};
use crate::support::copy_docks;
use crate::World;

/// One meter row from 0 to 6 at base 0, with its `hidden` key when it has one.
fn meter_row(id: &str, hidden: Option<bool>) -> String {
    let hidden = hidden.map_or(String::new(), |hidden| format!("    hidden: {hidden}\n"));
    format!("  - id: {id}\n    min: 0\n    base_cap: 6\n    max: 6\n    base: 0\n{hidden}")
}

/// The pack this scenario already set up, or a fresh docks copy.
fn pack(world: &mut World) -> &Path {
    world.pack.get_or_insert_with(|| copy_docks("holders"))
}

/// Write `story/<id>.yaml` with these extra keys and an empty rule list to append to.
fn write_story(world: &mut World, id: &str, keys: &str) {
    let dir = pack(world).join("story");
    fs::create_dir_all(&dir).unwrap();
    let text = format!("schema: story\nid: {id}\n{keys}rules:\n");
    fs::write(dir.join(format!("{id}.yaml")), text).unwrap();
}

/// Append a change rule that runs `effect` each time pc_rowan gains lamp oil.
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

/// Write `world/places/loft.md`, above the shop, with these extra keys.
fn write_loft(world: &mut World, keys: &str) {
    let text = format!(
        "---\nschema: place\nid: loft\ndisplay_name: Loft\nfacts:\n  - id: fact_loft_dust\n    text: Dust lies on the boards.\nexits:\n  - to: docks_shop\n    name: ladder\n{keys}---\nThe loft is over the shop.\n"
    );
    fs::write(pack(world).join("world/places/loft.md"), text).unwrap();
}

fn meter_effect(who: &str, key: &str) -> String {
    format!("      - meter:\n          who: {who}\n          key: {key}\n          add: 1\n")
}

fn flag_effect(who: &str, key: &str) -> String {
    format!("      - flag:\n          who: {who}\n          key: {key}\n          value: true\n")
}

/// Add verb `verb` to `rules/verbs.yaml` with these effects.
fn add_verb_doing(pack: &Path, verb: &str, effects: &str) {
    add_verb(
        pack,
        &format!("\n  - id: {verb}\n    when:\n      verb: {verb}\n    do:\n{effects}"),
    );
}

/// Put pc_rowan in the loft, seated, and Mira in the shop.
fn stand_in_the_loft(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    edit_table(world, &pack, |now, seated| {
        now.make_record(&spookery_engine::Holder::Place("loft".to_string()))
            .unwrap();
        set_place(now, "pc_rowan", "loft");
        set_place(now, "mira", "docks_shop");
        sit(seated, &["pc_rowan"]);
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
        &format!("meters:\n{}", meter_row(&key, None)),
    );
}

#[given(expr = "story\\/{word}.yaml declares {string} with hidden false")]
fn story_declares_a_shown_meter(world: &mut World, story: String, key: String) {
    let keys = format!("meters:\n{}", meter_row(&key, Some(false)));
    write_story(world, &story, &keys);
}

#[given(expr = "pc_rowan in the shop and pc_wren in the street are seated")]
fn rowan_and_wren_are_seated_apart(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    edit_table(world, &pack, |now, seated| {
        assert_eq!(now.actors["pc_rowan"].place, "docks_shop");
        assert_eq!(now.actors["pc_wren"].place, "docks_street");
        sit(seated, &["pc_rowan", "pc_wren"]);
    });
}

#[then(expr = "pc_rowan's and pc_wren's change lines show dread")]
fn rowan_and_wren_see_dread(world: &mut World) {
    for id in ["pc_rowan", "pc_wren"] {
        let seen = seen_of(world, id);
        assert!(
            seen.contains(&"story_meter ending dread".to_string()),
            "{id}: {seen:?}"
        );
    }
}

#[then(expr = "it refuses the story's {word} meter as {string}")]
fn it_refuses_the_story_meter(world: &mut World, key: String, why: String) {
    assert_refused(
        world,
        &["story/ending.yaml", &format!("meter '{key}' {why}")],
    );
}

#[then(expr = "the GM sees both light and draft")]
fn the_gm_sees_light_and_draft(world: &mut World) {
    for key in ["place_meter loft light", "place_meter loft draft"] {
        assert_gm_saw(world, key);
    }
}

#[given(expr = "story\\/{word}.yaml declares the flag {string}")]
fn story_declares_a_flag(world: &mut World, story: String, flag: String) {
    write_story(world, &story, &format!("flags:\n  - {flag}\n"));
}

#[given(expr = "story\\/{word}.yaml declares no flags")]
fn story_declares_no_flags(world: &mut World, story: String) {
    write_story(world, &story, "");
}

#[given(expr = "a rule in story\\/{word}.yaml adds 1 to who {string} meter {word}")]
fn a_story_rule_adds(world: &mut World, story: String, who: String, key: String) {
    append_story_rule(pack(world), &story, &meter_effect(&who, &key));
}

#[given(expr = "a rule in story\\/{word}.yaml sets who {string} flag {word} to true")]
fn a_story_rule_sets(world: &mut World, story: String, who: String, key: String) {
    append_story_rule(pack(world), &story, &flag_effect(&who, &key));
}

#[given(expr = "a rule sets who {string} flag {word}")]
fn a_verb_sets(world: &mut World, who: String, key: String) {
    add_verb_doing(pack(world), "set_mark", &flag_effect(&who, &key));
}

#[given(expr = "a rule sets who {string} flag {word} to true")]
fn a_shared_rule_sets(world: &mut World, who: String, key: String) {
    let rule = format!(
        "\n  - id: on_oil\n    when:\n      change:\n        item: lamp_oil\n        gained: true\n    do:\n{}",
        flag_effect(&who, &key)
    );
    add_verb(pack(world), &rule);
}

#[given(expr = "a rule adds 1 to who {string} meter {word}")]
fn a_rule_adds(world: &mut World, who: String, key: String) {
    match who.strip_prefix("story:") {
        Some(story) => append_story_rule(pack(world), story, &meter_effect(&who, &key)),
        None => add_verb_doing(pack(world), "bump", &meter_effect(&who, &key)),
    }
}

#[when(expr = "the rule runs")]
fn the_rule_runs(world: &mut World) {
    let take = intent("take", "pc_rowan", "take", Some("lamp_oil"));
    migrate_and_play(world, vec![take]);
}

#[then(expr = "the GM sees dread change on the ending story")]
fn the_gm_sees_dread(world: &mut World) {
    assert_gm_saw(world, "story_meter ending dread");
    let now = state_of(world.pack.as_deref().unwrap());
    let ending = spookery_engine::Holder::Story("ending".to_string());
    assert_eq!(now.meters.get(&ending, "dread").unwrap().offset, 1);
}

#[then(expr = "no player's view lists dread")]
fn no_view_lists_dread(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let opened = Opened::peek(&pack).unwrap();
    for id in ["pc_rowan", "pc_wren", "pc_harbormouse"] {
        let view = opened.perceive(&Presence::default(), id).unwrap();
        let mut listed = view.meters.iter().chain(&view.world_meters);
        assert!(listed.all(|meter| meter.id != "dread"), "{id}");
        let seen = seen_of(world, id);
        assert!(seen.iter().all(|key| !key.contains("dread")), "{seen:?}");
    }
}

#[then(expr = "a rule testing who {string} flag {word} is true sees it as set")]
fn a_rule_tests_the_flag(world: &mut World, who: String, key: String) {
    let pack = world.pack.clone().unwrap();
    add_verb(
        &pack,
        &format!(
            "\n  - id: test_mark\n    when:\n      verb: test_mark\n    if:\n      - flag:\n          who: {who}\n          key: {key}\n          is: true\n    do:\n{}",
            flag_effect("world", "mark_seen")
        ),
    );
    migrate_and_play(world, vec![intent("test", "pc_rowan", "test_mark", None)]);
    assert!(state_of(&pack).has_flag(&spookery_engine::Holder::World, "mark_seen"));
}

#[then(expr = "the GM sees the flag change on the siege story")]
fn the_gm_sees_the_siege_flag(world: &mut World) {
    assert_gm_saw(world, "story_flag siege walls_breached");
}

#[then(expr = "it refuses the rule, saying the siege story does not declare walls_breached")]
fn it_refuses_the_siege_flag(world: &mut World) {
    assert_refused(
        world,
        &[
            "rules/verbs.yaml",
            "rule 'set_mark' names flag 'walls_breached', but story:siege does not declare it",
        ],
    );
}

#[then(expr = "it refuses the rule, saying only the ending story carries dread")]
fn it_refuses_the_prologue_meter(world: &mut World) {
    assert_refused(
        world,
        &[
            "story/prologue.yaml",
            "rule 'on_oil' names meter 'dread', but only story:ending carries meter 'dread'",
        ],
    );
}

#[given(expr = "world\\/places\\/loft.md declares the flag {string}")]
fn loft_declares_a_flag(world: &mut World, flag: String) {
    write_loft(world, &format!("flags:\n  - {flag}\n"));
}

#[given(expr = "world\\/places\\/loft.md declares no meters")]
fn loft_declares_no_meters(world: &mut World) {
    write_loft(world, "");
}

#[given(
    expr = "world\\/places\\/loft.md declares {string} with hidden false and {string} with no hidden key"
)]
fn loft_declares_shown_and_hidden(world: &mut World, shown: String, hidden: String) {
    let rows = meter_row(&shown, Some(false)) + &meter_row(&hidden, None);
    write_loft(world, &format!("meters:\n{rows}"));
}

#[then(expr = "the GM sees the flag change on the loft")]
fn the_gm_sees_the_loft_flag(world: &mut World) {
    assert_gm_saw(world, "place_flag loft lamp_lit");
}

#[then(expr = "it refuses the rule, saying the loft does not declare light")]
fn it_refuses_the_loft_meter(world: &mut World) {
    assert_refused(
        world,
        &[
            "rules/verbs.yaml",
            "rule 'bump' names meter 'light', but place:loft carries no meter 'light'",
        ],
    );
}

#[given(expr = "pc_rowan stands in the loft and Mira stands in the shop")]
fn rowan_in_the_loft_mira_in_the_shop(world: &mut World) {
    stand_in_the_loft(world);
}

#[given(expr = "pc_rowan stands in the loft")]
fn rowan_in_the_loft(world: &mut World) {
    stand_in_the_loft(world);
}

#[when(expr = "a rule adds 1 to who {string} meter light and meter draft")]
fn a_rule_adds_light_and_draft(world: &mut World, who: String) {
    let pack = world.pack.clone().unwrap();
    let effects = meter_effect(&who, "light") + &meter_effect(&who, "draft");
    add_verb_doing(&pack, "air_out", &effects);
    edit_table(world, &pack, |_, seated| sit(seated, &["pc_rowan"]));
    migrate_and_play(world, vec![intent("air", "pc_rowan", "air_out", None)]);
}

#[when(expr = "a rule sets who {string} flag {word} to true")]
fn a_verb_sets_and_runs(world: &mut World, who: String, key: String) {
    let pack = world.pack.clone().unwrap();
    add_verb_doing(&pack, "light_lamp", &flag_effect(&who, &key));
    edit_table(world, &pack, |_, seated| sit(seated, &["pc_rowan"]));
    migrate_and_play(world, vec![intent("light", "pc_rowan", "light_lamp", None)]);
}

#[then(expr = "pc_rowan's change line shows light and not draft")]
fn rowan_sees_light_not_draft(world: &mut World) {
    let seen = seen_of(world, "pc_rowan");
    assert!(
        seen.contains(&"place_meter loft light".to_string()),
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

#[then(expr = "pc_rowan's change line does not show it")]
fn rowan_does_not_see_the_flag(world: &mut World) {
    assert_gm_saw(world, "place_flag loft lamp_lit");
    let seen = seen_of(world, "pc_rowan");
    assert!(seen.iter().all(|key| !key.contains("lamp_lit")), "{seen:?}");
}
