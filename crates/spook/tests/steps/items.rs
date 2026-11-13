use spookery_engine::testing::state_of;
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::{faces, RollKey};
use spookery_engine::{component, entity, State};

use super::common::split_line;
use super::table;
use crate::support;
use crate::World;

fn keep_briar_busy(now: &mut State) {
    if let Some(briar) = now.actors().acts.get("briar") {
        let pending = briar.pending.clone();
        let mut actors = now.actors_mut();
        actors
            .get_mut::<component::Act>(&"briar".into())
            .set_busy(u64::MAX, pending)
            .unwrap();
    }
}

/// Give `actor` one more of `item`.
fn give(now: &mut State, actor: &str, item: &str) {
    let (actor, mut actors) = (actor.into(), now.actors_mut());
    let mut held = actors.get_mut::<component::Item>(&actor);
    held.shift(item, 1, None).unwrap();
}

fn hp(pack: &Path, actor: &str) -> i64 {
    let now = support::state_of(pack);
    let id = spookery_engine::entity::Id::Actor(actor.into());
    now.entities()
        .get::<component::Meter>(&id)
        .value("hp")
        .unwrap()
}

fn count(pack: &Path, actor: &str, item: &str) -> u32 {
    support::state_of(pack)
        .actors()
        .items
        .get(&(actor.into(), item.to_string()))
        .copied()
        .unwrap_or(0)
}

fn wielded(pack: &Path, actor: &str) -> Option<String> {
    support::state_of(pack).actors().wielded.get(actor).cloned()
}

/// One line at a table with no GM: queue it, then commit its round.
fn handle_move(
    pack: &Path,
    actor: &str,
    line: &str,
) -> Result<Vec<spookery_engine::RoundOutput>, spookery_engine::Refusal> {
    let mut world = super::rules::open(pack);
    let (verb, target) = split_line(line);
    let nobody = spookery_engine::Presence::default();
    world.queue_player_move(&nobody, &actor.into(), verb, target)?;
    let stamp = world.state().last_round_at.clone();
    Ok(world
        .commit_then_fast_forward(&nobody, &stamp)
        .unwrap_or_else(|err| panic!("{line}: {err}")))
}

fn read_first_round_line(pack: &Path) -> String {
    spookery_engine::testing::journal::read_record_line(pack, "round-0001")
        .unwrap_or_else(|err| panic!("{}", err.detail))
}

fn write_dagger(pack: &Path, rules: &str) {
    let path = pack.join("world/items/poisoned_dagger.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let section = if rules.is_empty() {
        "rules: []\n".to_string()
    } else {
        format!("rules:\n{rules}")
    };
    fs::write(
        path,
        format!(
            "---\nschema: item\nid: poisoned_dagger\ndisplay_name: Poisoned dagger\ntags:\n  - weapon\n{section}---\n\nA dark blade.\n"
        ),
    )
    .unwrap();
}

const DAGGER_ATTACK: &str = "
  - id: attack
    when:
      verb: attack
      target: actor
    if:
      - wielding:
          subject: self
          id: poisoned_dagger
    roll:
      check: strike
      threshold: 0
    success:
      - meter:
          of: target
          key: hp
          add: -3
";

const DAGGER_STRIKE: &str = "
  - id: strike
    when:
      verb: strike
      target: actor
    do:
      - meter:
          of: target
          key: hp
          add: -3
";

const BARE_HANDS_ATTACK: &str = "
  - id: attack
    when:
      verb: attack
      target: actor
    roll:
      check: strike
      threshold: 0
    success:
      - meter:
          of: target
          key: hp
          add: -1
";

fn add_attack_rules(pack: &Path, attack: &str) {
    write_dagger(pack, attack);
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(BARE_HANDS_ATTACK);
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
}

fn hold_dagger(now: &mut State, wield: bool) {
    now.actors_mut()
        .get_mut::<component::Place>(&"lark".into())
        .set("gate_yard")
        .unwrap();
    now.actors_mut()
        .get_mut::<component::Place>(&"nell".into())
        .set("gate_yard")
        .unwrap();
    keep_briar_busy(now);
    give(now, "lark", "poisoned_dagger");
    if wield {
        let mut actors = now.actors_mut();
        actors
            .get_mut::<component::WieldedItem>(&"lark".into())
            .set(Some("poisoned_dagger"))
            .unwrap();
    }
}

#[given(expr = "nell holds 1 tonic and has 3 of 10 hp")]
fn nell_holds_a_tonic(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "tonic");
    let meters = pack.join("rules/meters.yaml");
    let text = fs::read_to_string(&meters).unwrap();
    let next = text.replace("base_cap: 12\n    max: 16", "base_cap: 10\n    max: 10");
    assert_ne!(text, next, "hp meter was not narrowed");
    fs::write(&meters, next).unwrap();
    support::edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"nell".into())
            .set("gate_yard")
            .unwrap();
        keep_briar_busy(now);
        give(now, "nell", "tonic");
        support::edit_meter(now, "nell", "hp", |hp| hp.base = 3);
    });
    world.pack = Some(pack);
}

#[when(expr = "nell types {string}")]
fn nell_types(world: &mut World, line: String) {
    let pack = world.pack.clone().unwrap();
    match handle_move(&pack, "nell", &line) {
        Ok(_) => world.scratch = "ok".to_string(),
        Err(err) => world.scratch = err.code.as_str().to_string(),
    }
}

#[then(expr = "nell has 7 hp and 0 tonic")]
fn nell_is_healed_and_empty(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(hp(pack, "nell"), 7);
    assert_eq!(count(pack, "nell", "tonic"), 0);
}

#[then(expr = "{string} again is rejected with {word} by the engine")]
fn again_is_refused(world: &mut World, line: String, code: String) {
    let pack = world.pack.as_ref().unwrap();
    let err = match handle_move(pack, "nell", &line) {
        Err(err) => err,
        Ok(_) => panic!("the second use was accepted"),
    };
    assert_eq!(err.code.as_str(), code);
    assert_eq!(hp(pack, "nell"), 7);
    assert_eq!(count(pack, "nell", "tonic"), 0);
    assert_eq!(
        support::state_of(pack).round_index,
        1,
        "the refusal committed another round"
    );
}

#[given(expr = "docks_shop has 3 lamp_oil")]
fn docks_shop_has_3_lamp_oil(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "take");
    support::edit_save(&pack, |now| {
        let shop = entity::Id::Place("docks_shop".into());
        support::put_items(now, &shop, "lamp_oil", 3);
    });
    world.pack = Some(pack);
}

#[when(expr = "rowan takes lamp_oil")]
fn rowan_takes_oil(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    handle_move(&pack, "rowan", "take lamp_oil")
        .unwrap_or_else(|err| panic!("{}", err.code.as_str()));
}

#[then(expr = "docks_shop has 2 and rowan has 1")]
fn the_oil_moved(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let now = support::state_of(pack);
    assert_eq!(
        now.places()
            .items
            .get(&("docks_shop".into(), "lamp_oil".to_string()))
            .copied(),
        Some(2)
    );
    assert_eq!(
        now.actors()
            .items
            .get(&("rowan".into(), "lamp_oil".to_string()))
            .copied()
            .unwrap_or(0),
        1
    );
}

#[then(expr = "the journal targets item {word}, then exit {word}")]
fn the_journal_targets_item_then_exit(world: &mut World, item: String, exit: String) {
    let records = spookery_engine::testing::journal::read_records(world.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let targets: Vec<Value> = records
        .iter()
        .filter_map(|record| match record {
            spookery_engine::testing::journal::Record::Round(round) => Some(&round.intents),
            _ => None,
        })
        .flatten()
        .map(|intent| serde_json::to_value(&intent.target).unwrap())
        .collect();
    assert_eq!(
        targets,
        [
            serde_json::json!({ "item": item }),
            serde_json::json!({ "exit": exit })
        ]
    );
}

#[when(expr = "rowan types {string}")]
fn rowan_types(world: &mut World, line: String) {
    let pack = world.pack.clone().unwrap();
    world.scratch = match handle_move(&pack, "rowan", &line) {
        Ok(_) => "ok".to_string(),
        Err(err) => err.code.as_str().to_string(),
    };
}

#[given(expr = "lark holds 1 latch at a seated table")]
fn lark_holds_a_latch_at_a_table(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "use-latch");
    support::edit_save(&pack, |now| {
        keep_briar_busy(now);
        give(now, "lark", "latch");
    });
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "lark");
    world.pack = Some(pack);
}

#[when(expr = "lark plays {string}")]
fn lark_plays(world: &mut World, line: String) {
    let before = state_of(world.pack.as_ref().unwrap());
    world.scratch = format!("{} {}", before.world_time, before.round_index);
    let (_mark, added) = table::player_send(world, &line, "\"kind\":\"refused\"");
    world.player_added = added;
}

#[then(expr = "lark is refused with {word}")]
fn lark_is_refused(world: &mut World, code: String) {
    let row = world
        .player_added
        .lines()
        .find(|row| row.contains("\"kind\":\"refused\""))
        .unwrap_or_else(|| panic!("no refused frame\n{}", world.player_added));
    let message: Value = serde_json::from_str(row).unwrap();
    let frame = &message["body"];
    assert_eq!(frame["code"], code.as_str(), "{frame}");
    assert!(frame.get("detail").is_none(), "{frame}");
}

#[then(expr = "no round was spent and lark still holds 1 latch")]
fn no_round_was_spent(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let now = state_of(pack);
    assert_eq!(
        format!("{} {}", now.world_time, now.round_index),
        world.scratch,
        "a round was spent"
    );
    assert!(spookery_engine::testing::journal::list_round_keys(pack).is_empty());
    assert_eq!(
        now.actors()
            .items
            .get(&("lark".into(), "latch".to_string()))
            .copied()
            .unwrap_or(0),
        1
    );
}

#[then(expr = "the line is rejected with {word} and no round was written")]
fn the_line_is_refused_and_quiet(world: &mut World, code: String) {
    assert_eq!(world.scratch, code);
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(support::state_of(pack).round_index, 0);
    assert!(spookery_engine::testing::journal::list_round_keys(pack).is_empty());
}

#[given(expr = "docks_shop has no lamp_oil")]
fn the_shop_has_no_oil(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "take-none");
    support::edit_save(&pack, |now| {
        let shop = entity::Id::Place("docks_shop".into());
        support::put_items(now, &shop, "lamp_oil", 0);
    });
    world.pack = Some(pack);
}

#[when(expr = "rowan drops lamp_oil")]
fn rowan_drops_oil(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    handle_move(&pack, "rowan", "drop lamp_oil")
        .unwrap_or_else(|err| panic!("{}", err.code.as_str()));
}

fn assert_round_rejects_missing_item(pack: &Path) {
    let round: Value = serde_json::from_str(read_first_round_line(pack).trim()).unwrap();
    let rejected = round["refused"].as_array().unwrap();
    assert!(
        rejected.iter().any(|item| item["code"] == "MISSING_ITEM"),
        "{round}"
    );
}

#[then(expr = "the round names MISSING_ITEM and rowan still has no lamp_oil")]
fn the_drop_was_missing(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    assert_round_rejects_missing_item(pack);
    let now = support::state_of(pack);
    assert_eq!(
        now.actors()
            .items
            .get(&("rowan".into(), "lamp_oil".to_string()))
            .copied()
            .unwrap_or(0),
        0
    );
    assert_eq!(
        now.places()
            .items
            .get(&("docks_shop".into(), "lamp_oil".to_string()))
            .copied(),
        Some(3)
    );
}

#[given(expr = "save.yaml lists an item with no definition")]
fn the_save_lists_an_unknown_item(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "unknown-item");
    support::edit_save(&pack, |now| give(now, "rowan", "ghost"));
    world.pack = Some(pack);
}

#[then(expr = "spook check reports an unknown item diagnostic")]
fn check_reports_an_unknown_item(world: &mut World) {
    let pack = support::pack_arg(world.pack.as_ref().unwrap());
    let output = support::run_spook(&["check", &pack]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    let report = format!("{}{}", world.stdout, world.stderr);
    assert!(report.contains("unknown item 'ghost'"), "{report}");
}

#[given(expr = "lark holds a poisoned_dagger tagged weapon")]
fn lark_holds_a_dagger(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "wield");
    write_dagger(&pack, "");
    support::edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"lark".into())
            .set("gate_yard")
            .unwrap();
        keep_briar_busy(now);
        give(now, "lark", "poisoned_dagger");
    });
    world.pack = Some(pack);
}

#[when(expr = "lark types {string}")]
fn lark_types(world: &mut World, line: String) {
    let pack = world.pack.clone().unwrap_or_else(|| {
        let pack = support::copy_pack("sample-d20", "wield-tonic");
        support::edit_save(&pack, |now| {
            keep_briar_busy(now);
        });
        world.pack = Some(pack.clone());
        pack
    });
    match handle_move(&pack, "lark", &line) {
        Ok(_) => {}
        Err(err) => world.scratch = err.code.as_str().to_string(),
    }
    world.player_added = line;
}

#[then(expr = "lark's wielded slot is poisoned_dagger")]
fn lark_wields_the_dagger(world: &mut World) {
    assert_eq!(
        wielded(world.pack.as_ref().unwrap(), "lark").as_deref(),
        Some("poisoned_dagger")
    );
}

#[then(expr = "it is rejected with NOT_A_WEAPON and the slot is unchanged")]
fn tonic_is_not_a_weapon(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let round: Value = serde_json::from_str(read_first_round_line(pack).trim()).unwrap();
    let rejected = round["refused"].as_array().unwrap();
    assert!(
        rejected.iter().any(|item| item["code"] == "NOT_A_WEAPON"),
        "{round}"
    );
    assert_eq!(wielded(pack, "lark"), None);
}

#[given(expr = "lark wields poisoned_dagger")]
fn lark_wields_for_attack(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "armed");
    add_attack_rules(&pack, DAGGER_ATTACK);
    support::edit_save(&pack, |now| hold_dagger(now, true));
    world.pack = Some(pack);
}

#[given(expr = "lark wields a poisoned_dagger with a strike rule")]
fn lark_wields_for_strike(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "armed-strike");
    write_dagger(&pack, DAGGER_STRIKE);
    support::edit_save(&pack, |now| hold_dagger(now, true));
    world.pack = Some(pack);
}

#[given(expr = "lark wields nothing")]
fn lark_wields_nothing(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "bare");
    add_attack_rules(&pack, DAGGER_ATTACK);
    support::edit_save(&pack, |now| hold_dagger(now, false));
    world.pack = Some(pack);
}

#[when(expr = "lark attacks nell and hits")]
fn lark_attacks_and_hits(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    handle_move(&pack, "lark", "attack nell").unwrap_or_else(|err| panic!("{}", err.code.as_str()));
}

#[when(expr = "lark attacks nell")]
fn lark_attacks(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    handle_move(&pack, "lark", "attack nell").unwrap_or_else(|err| panic!("{}", err.code.as_str()));
}

#[then(expr = "item:poisoned_dagger\\/{word} ran, not verb:{word}")]
fn the_dagger_rule_ran(world: &mut World, verb: String, shared: String) {
    let pack = world.pack.as_ref().unwrap();
    let round: Value = serde_json::from_str(read_first_round_line(pack).trim()).unwrap();
    let steps = round["extra_steps"].to_string();
    assert!(
        steps.contains(&format!("item:poisoned_dagger/{verb}")),
        "{steps}"
    );
    assert!(!steps.contains(&format!("verb:{shared}")), "{steps}");
    assert_eq!(hp(pack, "nell"), 5);
}

#[then(expr = "verb:attack ran")]
fn bare_hands_ran(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let round: Value = serde_json::from_str(read_first_round_line(pack).trim()).unwrap();
    let steps = round["extra_steps"].to_string();
    assert!(steps.contains("verb:attack"), "{steps}");
    assert!(!steps.contains("item:poisoned_dagger/attack"), "{steps}");
    assert_eq!(hp(pack, "nell"), 7);
}

#[when(expr = "lark drops poisoned_dagger")]
fn lark_drops_the_dagger(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    handle_move(&pack, "lark", "drop poisoned_dagger")
        .unwrap_or_else(|err| panic!("{}", err.code.as_str()));
}

#[then(expr = "lark's wielded slot is empty in the same round")]
fn the_slot_clears_in_the_round(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let now = support::state_of(pack);
    assert_eq!(now.round_index, 1);
    assert_eq!(now.actors().wielded.get("lark"), None);
    assert_eq!(
        now.actors()
            .items
            .get(&("lark".into(), "poisoned_dagger".to_string()))
            .copied()
            .unwrap_or(0),
        0
    );
    assert_eq!(
        now.places()
            .items
            .get(&("gate_yard".into(), "poisoned_dagger".to_string()))
            .copied(),
        Some(1)
    );
}

#[given(expr = "nell resists the poison with a seeded Constitution roll of 15 vs 12")]
fn nell_resists_on_a_seeded_roll(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "resist");
    let face = faces(
        &RollKey {
            world_id: "sample-d20".to_string(),
            round_index: 1,
            rule_id: "item:poisoned_dagger/attack".to_string(),
            subject_id: Some("nell".into()),
            index: 0,
        },
        0,
        1,
        20,
    )
    .unwrap()[0];
    let stat = 15 - i64::from(face);
    let meters = fs::read_to_string(pack.join("rules/meters.yaml")).unwrap();
    fs::write(
        pack.join("rules/meters.yaml"),
        format!(
            "{meters}  - id: constitution\n    min: -20\n    base_cap: 20\n    max: 20\n    base: 0\n"
        ),
    )
    .unwrap();
    let checks = fs::read_to_string(pack.join("rules/checks.yaml")).unwrap();
    fs::write(
        pack.join("rules/checks.yaml"),
        format!(
            "{checks}  - id: constitution\n    dice:\n      count: 1\n      sides: 20\n    meter: constitution\n"
        ),
    )
    .unwrap();
    let nell = fs::read_to_string(pack.join("cast/pc-nell.md")).unwrap();
    let next = nell.replace(
        "  eligible: false\n",
        "  eligible: false\nflags:\n    - poisoned\n",
    );
    assert_ne!(nell, next);
    fs::write(pack.join("cast/pc-nell.md"), next).unwrap();
    write_dagger(
        &pack,
        "
  - id: attack
    when:
      verb: attack
      target: actor
    if:
      - wielding:
          subject: self
          id: poisoned_dagger
    roll:
      check: strike
      threshold: 0
    success:
      - meter:
          of: target
          key: hp
          add: -3
      - meter:
          of: target
          key: hp
          add: -2
          resist:
            subject: target
            check: constitution
            threshold: 12
            resisted:
              - narrate:
                  to: here
                  text: The poison burns, then fades.
      - flag:
          of: target
          key: poisoned
          value: true
          resist: same
",
    );
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(BARE_HANDS_ATTACK);
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
    support::edit_save(&pack, |now| {
        hold_dagger(now, true);
        support::edit_meter(now, "nell", "hp", |hp| hp.base = 10);
        let nell = entity::Id::Actor("nell".into());
        support::put_meter(now, &nell, "constitution", stat);
    });
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    world.pack = Some(pack);
    world.scratch = stat.to_string();
}

#[when(expr = "lark hits nell with poisoned_dagger")]
fn lark_hits_with_the_dagger(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let outputs = handle_move(&pack, "lark", "attack nell")
        .unwrap_or_else(|err| panic!("{}", err.code.as_str()));
    let output = &outputs[0];
    let nell = output.reports.iter().find_map(|report| match report {
        spookery_engine::MoveReport::Rolled { actor_id, roll } if actor_id == "nell" => {
            Some(roll.clone())
        }
        _ => None,
    });
    let Some(nell) = nell else {
        panic!("nell did not roll: {:?}", output.reports);
    };
    assert_eq!(
        nell.total, 15,
        "stat {} face {:?}",
        world.scratch, nell.faces
    );
    assert_eq!(nell.target, 12);
    assert_eq!(output.reports.len(), 2, "{:?}", output.reports);
}

#[then(expr = "nell loses 3 hp, is not poisoned, sees {string}, and the round shows no refusal")]
fn the_poison_fades(world: &mut World, text: String) {
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(hp(pack, "nell"), 7);
    let poisoned = ("nell".into(), "poisoned".to_string());
    assert!(!support::state_of(pack)
        .actors()
        .flags
        .contains_key(&poisoned));
    let round: Value = serde_json::from_str(read_first_round_line(pack).trim()).unwrap();
    assert!(round["refused"].as_array().unwrap().is_empty(), "{round}");
    let effects = round["effects"].to_string();
    assert!(effects.contains(&text), "{effects}");
    assert!(effects.contains("nell"), "{effects}");
}

#[then(expr = "spook verify reproduces both rolls")]
fn verify_reproduces_both_rolls(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    spookery_engine::verify_pack(pack, None).unwrap_or_else(|err| panic!("{err:?}"));
}
