use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_kernel::meter;
use spookery_kernel::model::Now;
use spookery_kernel::rules::{faces, RollKey};
use spookery_kernel::store::codec::{emit_save, load_save};

use crate::support::{self, copy_dir};
use crate::World;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_pack(name: &str, label: &str) -> PathBuf {
    let pack = std::env::temp_dir().join(format!(
        "spook-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&repo().join("packs").join(name), &pack);
    pack
}

fn read_now(pack: &Path) -> Now {
    load_save(&fs::read_to_string(pack.join("state/save.yaml")).unwrap()).unwrap()
}

fn save(pack: &Path, edit: impl FnOnce(&mut Now)) {
    let mut now = read_now(pack);
    edit(&mut now);
    now.pack_rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    fs::write(pack.join("state/save.yaml"), emit_save(&now).unwrap()).unwrap();
}

fn place_of(now: &mut Now, actor: &str, place: &str) {
    let from = now.actors.get(actor).unwrap().place().to_string();
    if from == place {
        return;
    }
    if let Some(live) = now.places.get_mut(&from) {
        live.actors.retain(|id| id != actor);
    }
    let actors = &mut now.places.get_mut(place).unwrap().actors;
    actors.push(actor.to_string());
    actors.sort();
    actors.dedup();
    *now.actors.get_mut(actor).unwrap().place_mut() = place.to_string();
}

fn quiet(now: &mut Now) {
    if let Some(briar) = now.actors.get_mut("npc_briar") {
        briar.act_mut().busy_until = Some(u64::MAX);
    }
}

fn hp(pack: &Path, actor: &str) -> i64 {
    let now = read_now(pack);
    let canon = spookery_kernel::canon::load_canon(pack).unwrap();
    let def = canon.meters.get("hp").unwrap();
    let live = now.actors.get(actor).unwrap().meters().get("hp").unwrap();
    meter::current(def, live).unwrap()
}

fn count(pack: &Path, actor: &str, item: &str) -> u32 {
    read_now(pack).actors.get(actor).unwrap().count(item)
}

fn wielded(pack: &Path, actor: &str) -> Option<String> {
    read_now(pack)
        .actors
        .get(actor)
        .and_then(|actor| actor.wielded().map(str::to_string))
}

fn play(
    pack: &Path,
    actor: &str,
    line: &str,
) -> Result<spookery::host::MoveOutcome, spookery_kernel::rules::gate::Miss> {
    let mut world = spookery_kernel::World::open(pack).unwrap_or_else(|err| {
        panic!(
            "open {line}: {}\n{}",
            err.token(),
            spookery_kernel::check_pack(pack)
                .iter()
                .map(|diag| diag.report_line())
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    spookery::host::handle_player_move(&mut world, actor, line, false)
}

fn tick_text(pack: &Path) -> String {
    fs::read_to_string(pack.join("state/journal/tick-0001.jsonl")).unwrap()
}

fn write_dagger(pack: &Path, attack: &str) {
    let path = pack.join("world/items/poisoned_dagger.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let rules = if attack.is_empty() {
        "rules: []\n".to_string()
    } else {
        format!("rules:\n{attack}")
    };
    fs::write(
        path,
        format!(
            "---\nschema: item\nid: poisoned_dagger\nname: Poisoned dagger\ntags:\n  - weapon\n{rules}---\n\nA dark blade.\n"
        ),
    )
    .unwrap();
}

const SLASH: &str = "
  - id: attack
    when:
      verb: attack
      target: actor
    if:
      - wielding:
          who: self
          id: poisoned_dagger
    roll:
      check: strike
      target: 0
    success:
      - meter:
          who: target
          key: hp
          add: -3
";

const BARE: &str = "
  - id: attack
    when:
      verb: attack
      target: actor
    roll:
      check: strike
      target: 0
    success:
      - meter:
          who: target
          key: hp
          add: -1
";

fn arm(pack: &Path, attack: &str) {
    write_dagger(pack, attack);
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(BARE);
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
}

fn hold_dagger(now: &mut Now, wield: bool) {
    place_of(now, "pc_lark", "gate_yard");
    place_of(now, "pc_nell", "gate_yard");
    quiet(now);
    now.actors
        .get_mut("pc_lark")
        .unwrap()
        .inventory_mut()
        .insert("poisoned_dagger".to_string(), 1);
    if wield {
        *now.actors.get_mut("pc_lark").unwrap().wielded_mut() = Some("poisoned_dagger".to_string());
    }
}

#[given(expr = "nell holds 1 tonic and has 3 of 10 hp")]
fn nell_holds_a_tonic(world: &mut World) {
    let pack = copy_pack("sample-d20", "tonic");
    let meters = pack.join("rules/meters.yaml");
    let text = fs::read_to_string(&meters).unwrap();
    let next = text.replace("base_cap: 12\n    max: 16", "base_cap: 10\n    max: 10");
    assert_ne!(text, next, "hp meter was not narrowed");
    fs::write(&meters, next).unwrap();
    save(&pack, |now| {
        place_of(now, "pc_nell", "gate_yard");
        quiet(now);
        now.actors
            .get_mut("pc_nell")
            .unwrap()
            .inventory_mut()
            .insert("tonic".to_string(), 1);
        now.actors
            .get_mut("pc_nell")
            .unwrap()
            .meters_mut()
            .get_mut("hp")
            .unwrap()
            .base = 3;
    });
    world.pack = Some(pack);
}

#[when(expr = "nell types {string}")]
fn nell_types(world: &mut World, line: String) {
    let pack = world.pack.clone().unwrap();
    match play(&pack, "pc_nell", &line) {
        Ok(_) => world.scratch = "ok".to_string(),
        Err(err) => world.scratch = err.token().to_string(),
    }
}

#[then(expr = "nell has 7 hp and 0 tonic")]
fn nell_is_healed_and_empty(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(hp(pack, "pc_nell"), 7);
    assert_eq!(count(pack, "pc_nell", "tonic"), 0);
}

#[then(expr = "{string} again is rejected with MISSING_ITEM by the kernel")]
fn again_is_missing_item(world: &mut World, line: String) {
    let pack = world.pack.as_ref().unwrap();
    let err = match play(pack, "pc_nell", &line) {
        Err(err) => err,
        Ok(_) => panic!("the second use was accepted"),
    };
    assert_eq!(err.token(), "MISSING_ITEM");
    assert_eq!(hp(pack, "pc_nell"), 7);
    assert_eq!(count(pack, "pc_nell", "tonic"), 0);
    assert_eq!(
        read_now(pack).tick_index,
        1,
        "the refusal committed another tick"
    );
}

#[given(expr = "docks_shop has 3 lamp_oil")]
fn the_shop_has_three_tins(world: &mut World) {
    let pack = copy_pack("oneshot-docks", "take");
    save(&pack, |now| {
        now.places
            .get_mut("docks_shop")
            .unwrap()
            .items
            .insert("lamp_oil".to_string(), 3);
    });
    world.pack = Some(pack);
}

#[when(expr = "rowan takes lamp_oil")]
fn rowan_takes_oil(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    play(&pack, "pc_rowan", "take lamp_oil").unwrap_or_else(|err| panic!("{}", err.token()));
}

#[then(expr = "docks_shop has 2 and rowan has 1")]
fn the_tin_moved(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let now = read_now(pack);
    assert_eq!(
        now.places
            .get("docks_shop")
            .unwrap()
            .items
            .get("lamp_oil")
            .copied(),
        Some(2)
    );
    assert_eq!(now.actors.get("pc_rowan").unwrap().count("lamp_oil"), 1);
}

#[when(expr = "rowan types {string}")]
fn rowan_types(world: &mut World, line: String) {
    let pack = world.pack.clone().unwrap();
    world.scratch = match play(&pack, "pc_rowan", &line) {
        Ok(_) => "ok".to_string(),
        Err(err) => err.token().to_string(),
    };
}

#[then(expr = "the line is rejected with MISSING_ITEM and no tick was written")]
fn the_line_is_missing_and_quiet(world: &mut World) {
    assert_eq!(world.scratch, "MISSING_ITEM");
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(read_now(pack).tick_index, 0);
    assert!(!pack.join("state/journal/tick-0001.jsonl").exists());
}

#[given(expr = "docks_shop has no lamp_oil")]
fn the_shop_has_no_oil(world: &mut World) {
    let pack = copy_pack("oneshot-docks", "take-none");
    save(&pack, |now| {
        now.places
            .get_mut("docks_shop")
            .unwrap()
            .items
            .remove("lamp_oil");
    });
    world.pack = Some(pack);
}

#[when(expr = "rowan drops lamp_oil")]
fn rowan_drops_oil(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    play(&pack, "pc_rowan", "drop lamp_oil").unwrap_or_else(|err| panic!("{}", err.token()));
}

fn tick_names_missing(pack: &Path) {
    let tick: Value = serde_json::from_str(tick_text(pack).trim()).unwrap();
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected.iter().any(|item| item["reason"] == "MISSING_ITEM"),
        "{tick}"
    );
}

#[then(expr = "the tick names MISSING_ITEM and the shop has no lamp_oil")]
fn the_take_was_missing(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    tick_names_missing(pack);
    let now = read_now(pack);
    assert_eq!(
        now.places
            .get("docks_shop")
            .unwrap()
            .items
            .get("lamp_oil")
            .copied(),
        None
    );
    assert_eq!(now.actors.get("pc_rowan").unwrap().count("lamp_oil"), 0);
}

#[then(expr = "the tick names MISSING_ITEM and rowan still has no lamp_oil")]
fn the_drop_was_missing(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    tick_names_missing(pack);
    let now = read_now(pack);
    assert_eq!(now.actors.get("pc_rowan").unwrap().count("lamp_oil"), 0);
    assert_eq!(
        now.places
            .get("docks_shop")
            .unwrap()
            .items
            .get("lamp_oil")
            .copied(),
        Some(3)
    );
}

#[given(expr = "now.yaml lists an item with no definition")]
fn the_save_lists_an_unknown_item(world: &mut World) {
    let pack = copy_pack("oneshot-docks", "unknown-item");
    save(&pack, |now| {
        now.actors
            .get_mut("pc_rowan")
            .unwrap()
            .inventory_mut()
            .insert("ghost".to_string(), 1);
    });
    world.pack = Some(pack);
}

#[then(expr = "spook check reports an unknown item diagnostic")]
fn check_reports_an_unknown_item(world: &mut World) {
    let pack = support::pack_arg(world.pack.as_ref().unwrap());
    let output = support::spook(&["check", &pack]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    let report = format!("{}{}", world.stdout, world.stderr);
    assert!(report.contains("unknown item 'ghost'"), "{report}");
}

#[given(expr = "lark holds a poisoned_dagger tagged weapon")]
fn lark_holds_a_dagger(world: &mut World) {
    let pack = copy_pack("sample-d20", "wield");
    write_dagger(&pack, "");
    save(&pack, |now| {
        place_of(now, "pc_lark", "gate_yard");
        quiet(now);
        now.actors
            .get_mut("pc_lark")
            .unwrap()
            .inventory_mut()
            .insert("poisoned_dagger".to_string(), 1);
    });
    world.pack = Some(pack);
}

#[when(expr = "lark types {string}")]
fn lark_types(world: &mut World, line: String) {
    let pack = world.pack.clone().unwrap_or_else(|| {
        let pack = copy_pack("sample-d20", "wield-tonic");
        save(&pack, |now| {
            quiet(now);
        });
        world.pack = Some(pack.clone());
        pack
    });
    match play(&pack, "pc_lark", &line) {
        Ok(_) => {}
        Err(err) => world.scratch = err.token().to_string(),
    }
    world.player_added = line;
}

#[then(expr = "lark's wielded slot is poisoned_dagger")]
fn lark_wields_the_dagger(world: &mut World) {
    assert_eq!(
        wielded(world.pack.as_ref().unwrap(), "pc_lark").as_deref(),
        Some("poisoned_dagger")
    );
}

#[then(expr = "it is rejected with NOT_A_WEAPON and the slot is unchanged")]
fn tonic_is_not_a_weapon(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let tick: Value = serde_json::from_str(tick_text(pack).trim()).unwrap();
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected.iter().any(|item| item["reason"] == "NOT_A_WEAPON"),
        "{tick}"
    );
    assert_eq!(wielded(pack, "pc_lark"), None);
}

#[given(expr = "lark wields poisoned_dagger")]
fn lark_wields_for_attack(world: &mut World) {
    let pack = copy_pack("sample-d20", "armed");
    arm(&pack, SLASH);
    save(&pack, |now| hold_dagger(now, true));
    world.pack = Some(pack);
}

#[given(expr = "lark wields nothing")]
fn lark_wields_nothing(world: &mut World) {
    let pack = copy_pack("sample-d20", "bare");
    arm(&pack, SLASH);
    save(&pack, |now| hold_dagger(now, false));
    world.pack = Some(pack);
}

#[when(expr = "lark attacks nell and hits")]
fn lark_attacks_and_hits(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    play(&pack, "pc_lark", "attack pc_nell").unwrap_or_else(|err| panic!("{}", err.token()));
}

#[when(expr = "lark attacks nell")]
fn lark_attacks(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    play(&pack, "pc_lark", "attack pc_nell").unwrap_or_else(|err| panic!("{}", err.token()));
}

#[then(expr = "item:poisoned_dagger\\/attack ran, not verb:attack")]
fn the_dagger_rule_ran(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let tick: Value = serde_json::from_str(tick_text(pack).trim()).unwrap();
    let trace = tick["tool_trace"].to_string();
    assert!(trace.contains("item:poisoned_dagger/attack"), "{trace}");
    assert!(!trace.contains("verb:attack"), "{trace}");
    assert_eq!(hp(pack, "pc_nell"), 5);
}

#[then(expr = "verb:attack ran")]
fn bare_hands_ran(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let tick: Value = serde_json::from_str(tick_text(pack).trim()).unwrap();
    let trace = tick["tool_trace"].to_string();
    assert!(trace.contains("verb:attack"), "{trace}");
    assert!(!trace.contains("item:poisoned_dagger/attack"), "{trace}");
    assert_eq!(hp(pack, "pc_nell"), 7);
}

#[when(expr = "lark drops poisoned_dagger")]
fn lark_drops_the_dagger(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    play(&pack, "pc_lark", "drop poisoned_dagger").unwrap_or_else(|err| panic!("{}", err.token()));
}

#[then(expr = "lark's wielded slot is empty in the same tick")]
fn the_slot_clears_in_the_tick(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let now = read_now(pack);
    assert_eq!(now.tick_index, 1);
    assert_eq!(now.actors.get("pc_lark").unwrap().wielded(), None);
    assert_eq!(
        now.actors.get("pc_lark").unwrap().count("poisoned_dagger"),
        0
    );
    assert_eq!(
        now.places
            .get("gate_yard")
            .unwrap()
            .items
            .get("poisoned_dagger")
            .copied(),
        Some(1)
    );
}

#[given(expr = "nell resists the poison with a seeded Constitution roll of 15 vs 12")]
fn nell_resists_on_a_seeded_roll(world: &mut World) {
    let pack = copy_pack("sample-d20", "resist");
    let face = faces(
        &RollKey {
            world_id: "sample-d20".to_string(),
            round_index: 1,
            rule_id: "attack".to_string(),
            subject_id: "pc_nell".to_string(),
            roll_index: 0,
        },
        1,
        20,
    )
    .unwrap()[0];
    let stat = 15 - i64::from(face);
    let meters = fs::read_to_string(pack.join("rules/meters.yaml")).unwrap();
    fs::write(
        pack.join("rules/meters.yaml"),
        format!(
            "{meters}  - id: constitution\n    min: -20\n    base_cap: 20\n    max: 20\n    start: 0\n"
        ),
    )
    .unwrap();
    let checks = fs::read_to_string(pack.join("rules/checks.yaml")).unwrap();
    fs::write(
        pack.join("rules/checks.yaml"),
        format!(
            "{checks}  - id: constitution\n    dice:\n      count: 1\n      sides: 20\n    stat: constitution\n"
        ),
    )
    .unwrap();
    let nell = fs::read_to_string(pack.join("cast/pc-nell.md")).unwrap();
    let next = nell.replace(
        "  eligible: false\n",
        "  eligible: false\n  flags:\n    - poisoned\n",
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
          who: self
          id: poisoned_dagger
    roll:
      check: strike
      target: 0
    success:
      - meter:
          who: target
          key: hp
          add: -3
      - meter:
          who: target
          key: hp
          add: -2
          resist:
            who: target
            check: constitution
            target: 12
            resisted:
              - narrate:
                  to: here
                  text: The poison burns, then fades.
      - flag:
          who: target
          key: poisoned
          value: true
          resist: same
",
    );
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(BARE);
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
    save(&pack, |now| {
        hold_dagger(now, true);
        now.actors
            .get_mut("pc_nell")
            .unwrap()
            .meters_mut()
            .get_mut("hp")
            .unwrap()
            .base = 10;
        now.actors.get_mut("pc_nell").unwrap().meters_mut().insert(
            "constitution".to_string(),
            spookery_kernel::meter::MeterLive {
                base: stat,
                offset: 0,
            },
        );
    });
    world.pack = Some(pack);
    world.scratch = stat.to_string();
}

#[when(expr = "lark hits nell with poisoned_dagger")]
fn lark_hits_with_the_dagger(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let played =
        play(&pack, "pc_lark", "attack pc_nell").unwrap_or_else(|err| panic!("{}", err.token()));
    let spookery::host::MoveOutcome::Committed { notices, .. } = played else {
        panic!("the hit stayed queued");
    };
    let nell = notices.iter().find_map(|notice| match notice {
        spookery_kernel::rules::RollReport::Rolled { actor_id, out } if actor_id == "pc_nell" => {
            Some(out.clone())
        }
        _ => None,
    });
    let Some(nell) = nell else {
        panic!("nell did not roll: {notices:?}");
    };
    assert_eq!(
        nell.total, 15,
        "stat {} face {:?}",
        world.scratch, nell.faces
    );
    assert_eq!(nell.target, 12);
    assert_eq!(notices.len(), 2, "{notices:?}");
}

#[then(expr = "nell loses 3 hp, is not poisoned, sees {string}, and the tick shows no refusal")]
fn the_poison_fades(world: &mut World, text: String) {
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(hp(pack, "pc_nell"), 7);
    assert!(!read_now(pack)
        .actors
        .get("pc_nell")
        .unwrap()
        .flags()
        .iter()
        .any(|flag| flag == "poisoned"));
    let tick: Value = serde_json::from_str(tick_text(pack).trim()).unwrap();
    assert!(tick["rejected"].as_array().unwrap().is_empty(), "{tick}");
    let effects = tick["effects"].to_string();
    assert!(effects.contains(&text), "{effects}");
    assert!(effects.contains("pc_nell"), "{effects}");
}

#[then(expr = "spook verify reproduces both rolls")]
fn verify_reproduces_both_rolls(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    spookery_round::verify_pack(pack, None).unwrap_or_else(|err| panic!("{err:?}"));
}
