use spookery_engine::testing::world_of;
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::roll;
use spookery_engine::{component, ecs, entity, MoveReport};

use super::common::resolve_id;
use super::host;
use super::rules::split_line;
use crate::support;
use crate::World;

/// Give `actor` one more of `item`.
fn give(now: &mut ecs::World, actor: &str, item: &str) {
    let (actor, mut actors) = (actor.into(), now.actors_mut());
    let mut items = actors.get_mut::<component::Item>(&actor);
    items.shift(item, 1, None).unwrap();
}

fn hp(pack: &Path, actor: &str) -> i64 {
    let now = support::world_of(pack);
    let id = spookery_engine::entity::Id::Actor(actor.into());
    now.entities()
        .get::<component::Meter>(&id)
        .value("hp")
        .unwrap()
}

fn count(pack: &Path, actor: &str, item: &str) -> u32 {
    support::world_of(pack)
        .actors()
        .get::<component::Item>(&actor.into())
        .get(item)
        .copied()
        .unwrap_or(0)
}

fn wielded(pack: &Path, actor: &str) -> Option<String> {
    let (now, actor) = (support::world_of(pack), actor.into());
    let wielded = now.actors().get::<component::WieldedItem>(&actor).get();
    wielded.map(String::from)
}

/// One line with the GM seat empty: queue it, then commit its round.
fn handle_move(
    pack: &Path,
    actor: &str,
    line: &str,
) -> Result<Vec<spookery_engine::round::Output>, spookery_engine::Refusal> {
    let mut session = super::rules::open(pack);
    let actor = resolve_id(session.canon(), actor);
    let (verb, target) = split_line(line);
    let nobody = spookery_engine::Presence::default();
    session.queue_player_move(&nobody, &actor.into(), verb, target)?;
    let stamp = session.world().last_round_at.clone();
    Ok(session
        .commit_then_fast_forward(&nobody, &stamp)
        .unwrap_or_else(|err| panic!("{line}: {err}")))
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
            "---\nschema: item\nid: poisoned_dagger\nname: Poisoned dagger\ntags:\n  - weapon\n{section}---\n\nA dark blade.\n"
        ),
    )
    .unwrap();
}

const DAGGER_ATTACK: &str = "
  - id: attack
    when:
      verb_id: attack
      target: actor
    if:
      - wielding:
          subject: self
          id: poisoned_dagger
    check:
      total: 1d20 + self.might
      success_at: 0
    success:
      - meter:
          of: target
          id: hp
          add: -3
";

const DAGGER_STRIKE: &str = "
  - id: strike
    when:
      verb_id: strike
      target: actor
    do:
      - meter:
          of: target
          id: hp
          add: -4
";

const BARE_HANDS_ATTACK: &str = "
  - id: attack
    when:
      verb_id: attack
      target: actor
    check:
      total: 1d20 + self.might
      success_at: 0
    success:
      - meter:
          of: target
          id: hp
          add: -1
";

fn add_attack_rules(pack: &Path, attack: &str) {
    write_dagger(pack, attack);
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(BARE_HANDS_ATTACK);
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
}

fn hold_dagger(now: &mut ecs::World, wield: bool) {
    now.actors_mut()
        .get_mut::<component::Place>(&"wren".into())
        .set("gate_yard")
        .unwrap();
    give(now, "harbormouse", "poisoned_dagger");
    if wield {
        let mut actors = now.actors_mut();
        actors
            .get_mut::<component::WieldedItem>(&"harbormouse".into())
            .set(Some("poisoned_dagger"))
            .unwrap();
    }
}

#[given(expr = "harbormouse holds 1 tonic and has 3 of 10 hp")]
fn harbormouse_holds_a_tonic(world: &mut World) {
    let pack = support::copy_pack("docks", "tonic");
    let meters = pack.join("rules/meters.yaml");
    let text = fs::read_to_string(&meters).unwrap();
    let next = text.replace("base_cap: 12\n    max: 16", "base_cap: 10\n    max: 10");
    assert_ne!(text, next, "hp meter was not narrowed");
    fs::write(&meters, next).unwrap();
    support::edit_save(&pack, |now| {
        support::edit_meter(now, "harbormouse", "hp", |hp| hp.base = 3);
    });
    world.pack = Some(pack);
}

#[when(regex = r#"^(?P<actor>[A-Z]\S*) types "(?P<text>.*)"$"#)]
fn actor_types(world: &mut World, actor: String, line: String) {
    let pack = world.pack.clone().unwrap();
    world.scratch = match handle_move(&pack, &actor, &line) {
        Ok(outputs) => {
            let refused = list_refusals(outputs.iter().flat_map(|output| &output.reports));
            world.summary = outputs.into_iter().flat_map(|o| o.changed_keys).collect();
            refused
        }
        Err(err) => err.code.as_str().to_string(),
    };
}

/// Each move `reports` told a player was refused, as `actor CODE`, joined by commas.
pub(crate) fn list_refusals<'r>(reports: impl IntoIterator<Item = &'r MoveReport>) -> String {
    let refused = reports.into_iter().filter_map(|report| match report {
        MoveReport::Refused { actor_id, refusal } => {
            Some(format!("{actor_id} {}", refusal.code.as_str()))
        }
        MoveReport::Rolled { .. } => None,
    });
    refused.collect::<Vec<_>>().join(", ")
}

#[then(expr = "harbormouse has 7 hp and 0 tonic")]
fn harbormouse_is_healed_and_empty(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(hp(pack, "harbormouse"), 7);
    assert_eq!(count(pack, "harbormouse", "tonic"), 0);
}

#[then(expr = "{string} again is rejected with {word} by the engine")]
fn again_is_refused(world: &mut World, line: String, code: String) {
    let pack = world.pack.as_ref().unwrap();
    let err = match handle_move(pack, "harbormouse", &line) {
        Err(err) => err,
        Ok(_) => panic!("the second use was accepted"),
    };
    assert_eq!(err.code.as_str(), code);
    assert_eq!(hp(pack, "harbormouse"), 7);
    assert_eq!(count(pack, "harbormouse", "tonic"), 0);
    assert_eq!(
        support::world_of(pack).round_index,
        1,
        "the refusal committed another round"
    );
}

#[given(expr = "docks_shop has 3 lamp_oil")]
fn docks_shop_has_3_lamp_oil(world: &mut World) {
    let pack = support::copy_pack("docks", "take");
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
    let now = support::world_of(pack);
    assert_eq!(
        now.places()
            .get::<component::Item>(&"docks_shop".into())
            .get("lamp_oil")
            .copied(),
        Some(2)
    );
    assert_eq!(
        now.actors()
            .get::<component::Item>(&"rowan".into())
            .get("lamp_oil")
            .copied()
            .unwrap_or(0),
        1
    );
}

#[given(expr = "harbormouse holds 1 latch at a seated table")]
async fn harbormouse_holds_a_latch_at_a_table(world: &mut World) {
    let pack = support::copy_pack("docks", "use-latch");
    support::edit_save(&pack, |now| give(now, "harbormouse", "latch"));
    host::listen(world, &pack).await;
    host::seat_actor(world, &pack, "harbormouse").await;
    world.pack = Some(pack);
}

#[when(expr = "harbormouse plays {string}")]
async fn harbormouse_plays(world: &mut World, line: String) {
    let before = world_of(world.pack.as_ref().unwrap());
    world.scratch = format!("{} {}", before.world_time, before.round_index);
    let (_mark, added) = host::player_send(world, &line, "\"kind\":\"refused\"").await;
    world.player_added = added;
}

#[then(expr = "harbormouse is refused with {word}")]
fn harbormouse_is_refused(world: &mut World, code: String) {
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

#[then(expr = "no round was spent and harbormouse still holds 1 latch")]
fn no_round_was_spent(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let now = world_of(pack);
    assert_eq!(
        format!("{} {}", now.world_time, now.round_index),
        world.scratch,
        "a round was spent"
    );
    assert_eq!(
        now.actors()
            .get::<component::Item>(&"harbormouse".into())
            .get("latch")
            .copied()
            .unwrap_or(0),
        1
    );
}

#[then(expr = "the line is rejected with {word} and no round was written")]
fn the_line_is_refused_and_quiet(world: &mut World, code: String) {
    assert_eq!(world.scratch, code);
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(support::world_of(pack).round_index, 0);
}

#[when(expr = "rowan drops lamp_oil")]
fn rowan_drops_oil(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let outputs = handle_move(&pack, "rowan", "drop lamp_oil")
        .unwrap_or_else(|err| panic!("{}", err.code.as_str()));
    world.scratch = list_refusals(outputs.iter().flat_map(|output| &output.reports));
}

#[then(expr = "rowan is told MISSING_ITEM and still has no lamp_oil")]
fn the_drop_was_missing(world: &mut World) {
    assert_eq!(world.scratch, "rowan MISSING_ITEM");
    let pack = world.pack.as_ref().unwrap();
    let now = support::world_of(pack);
    assert_eq!(
        now.actors()
            .get::<component::Item>(&"rowan".into())
            .get("lamp_oil")
            .copied()
            .unwrap_or(0),
        0
    );
    assert_eq!(
        now.places()
            .get::<component::Item>(&"docks_shop".into())
            .get("lamp_oil")
            .copied(),
        Some(3)
    );
}

#[given(expr = "harbormouse holds a poisoned_dagger tagged weapon")]
fn harbormouse_holds_a_dagger(world: &mut World) {
    let pack = support::copy_pack("docks", "wield");
    write_dagger(&pack, "");
    support::edit_save(&pack, |now| give(now, "harbormouse", "poisoned_dagger"));
    world.pack = Some(pack);
}

#[then(expr = "harbormouse's wielded slot is poisoned_dagger")]
fn harbormouse_wielded_slot_is_poisoned_dagger(world: &mut World) {
    assert_eq!(
        wielded(world.pack.as_ref().unwrap(), "harbormouse").as_deref(),
        Some("poisoned_dagger")
    );
}

#[then(expr = "it is rejected with NOT_A_WEAPON and the slot is unchanged")]
fn tonic_is_not_a_weapon(world: &mut World) {
    assert_eq!(world.scratch, "harbormouse NOT_A_WEAPON");
    assert_eq!(wielded(world.pack.as_ref().unwrap(), "harbormouse"), None);
}

#[given(expr = "harbormouse wields poisoned_dagger")]
fn harbormouse_wields_poisoned_dagger(world: &mut World) {
    let pack = support::copy_pack("docks", "armed");
    add_attack_rules(&pack, DAGGER_ATTACK);
    support::edit_save(&pack, |now| hold_dagger(now, true));
    world.pack = Some(pack);
}

#[given(expr = "harbormouse wields a poisoned_dagger with a strike rule")]
fn harbormouse_wields_a_poisoned_dagger_with_a_strike_rule(world: &mut World) {
    let pack = support::copy_pack("docks", "armed-strike");
    write_dagger(&pack, DAGGER_STRIKE);
    support::edit_save(&pack, |now| hold_dagger(now, true));
    world.pack = Some(pack);
}

#[given(expr = "harbormouse wields nothing")]
fn harbormouse_wields_nothing(world: &mut World) {
    let pack = support::copy_pack("docks", "bare");
    add_attack_rules(&pack, DAGGER_ATTACK);
    support::edit_save(&pack, |now| hold_dagger(now, false));
    world.pack = Some(pack);
}

#[when(expr = "harbormouse attacks wren")]
fn harbormouse_attacks_wren(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    handle_move(&pack, "harbormouse", "attack wren")
        .unwrap_or_else(|err| panic!("{}", err.code.as_str()));
}

#[when(expr = "harbormouse drops poisoned_dagger")]
fn harbormouse_drops_poisoned_dagger(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    handle_move(&pack, "harbormouse", "drop poisoned_dagger")
        .unwrap_or_else(|err| panic!("{}", err.code.as_str()));
}

#[then(expr = "harbormouse's wielded slot is empty in the same round")]
fn the_slot_clears_in_the_round(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let now = support::world_of(pack);
    assert_eq!(now.round_index, 1);
    assert_eq!(wielded(pack, "harbormouse"), None);
    assert_eq!(
        now.actors()
            .get::<component::Item>(&"harbormouse".into())
            .get("poisoned_dagger")
            .copied()
            .unwrap_or(0),
        0
    );
    assert_eq!(
        now.places()
            .get::<component::Item>(&"gate_yard".into())
            .get("poisoned_dagger")
            .copied(),
        Some(1)
    );
}

#[given(expr = "wren saves against the poison with a seeded Constitution roll of 15 vs 12")]
fn wren_rolls_a_nested_check_on_a_seeded_roll(world: &mut World) {
    let pack = support::copy_pack("docks", "nested-check");
    let face = roll::faces(
        &roll::Key {
            seed: "docks-001".to_string(),
            round_index: 1,
            rule_id: "item:poisoned_dagger/attack".to_string(),
            subject_id: Some("harbormouse".into()),
            index: 1,
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
    write_dagger(
        &pack,
        "
  - id: attack
    when:
      verb_id: attack
      target: actor
    if:
      - wielding:
          subject: self
          id: poisoned_dagger
    check:
      total: 1d20 + self.might
      success_at: 0
    success:
      - meter:
          of: target
          id: hp
          add: -3
      - check:
          by: target
          total: 1d20 + target.constitution
          success_at: 12
          success:
            - narrate:
                to: here
                text: The poison burns, then fades.
          miss:
            - meter:
                of: target
                id: hp
                add: -2
            - flag:
                of: target
                id: poisoned
                value: true
",
    );
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(BARE_HANDS_ATTACK);
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
    support::edit_save(&pack, |now| {
        hold_dagger(now, true);
        support::edit_meter(now, "wren", "hp", |hp| hp.base = 10);
        let wren = entity::Id::Actor("wren".into());
        support::put_meter(now, &wren, "constitution", stat);
    });
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    world.pack = Some(pack);
}

#[when(expr = "harbormouse hits a seated wren with poisoned_dagger")]
async fn harbormouse_hits_a_seated_wren_with_the_dagger(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    host::serve_in_process(world, &pack).await;
    host::seat_actor(world, &pack, "harbormouse").await;
    host::seat_actor(world, &pack, "wren").await;
    world.wren_mark = host::player_client(world, "wren").seat.mark();
    let _ = host::actor_send(world, "harbormouse", "attack wren", "\"kind\":\"check\"").await;
}

#[then(expr = "wren is told she saved")]
async fn wren_is_told_her_nested_check_succeeded(world: &mut World) {
    let needle = r#"{"kind":"check","outcome":"success"}"#;
    let added = host::wait_actor_from(world, "wren", world.wren_mark, needle).await;
    assert_eq!(added.matches(r#""kind":"check""#).count(), 1, "{added}");
}

#[then(expr = "wren loses 3 hp, is not poisoned and sees {string}")]
async fn the_poison_fades(world: &mut World, text: String) {
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(hp(pack, "wren"), 7);
    assert!(!support::world_of(pack)
        .actors()
        .get::<component::Flag>(&"wren".into())
        .contains("poisoned"));
    host::wait_actor_from(world, "wren", world.wren_mark, &text).await;
}

#[then(expr = "spook verify reproduces both rolls")]
fn verify_reproduces_both_rolls(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    spookery_engine::verify_pack(pack, None).unwrap_or_else(|err| panic!("{err:?}"));
}
