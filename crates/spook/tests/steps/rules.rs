use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_kernel::api::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_kernel::api::model::{Now, PlaceLive};
use spookery_kernel::api::rules::gate;
use spookery_kernel::api::rules::{faces, RollKey, RollReport};
use spookery_kernel::api::store::codec::{emit_save, load_save};
use spookery_kernel::api::Reason;
use spookery_round::{play, RoundCause};

use super::table;
use crate::support::{copy_pack, spook, temp_pack};
use crate::World;
#[given(expr = "a save with rules_rev 6 and an engine at 7")]
fn a_save_with_rules_rev_6_and_an_engine_at_7(world: &mut World) {
    assert_eq!(
        spookery_kernel::api::store::RULES_REV,
        7,
        "the engine rules version is 7"
    );
    let pack = crate::support::temp_pack("rules-rev");
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read save: {err}"));
    let stamped = text.replacen("rules_rev: 7\n", "rules_rev: 6\n", 1);
    assert_ne!(stamped, text, "save did not carry rules_rev 7\n{text}");
    fs::write(&path, stamped).unwrap_or_else(|err| panic!("write save: {err}"));
    world.pack = Some(pack);
}

/// The save shape rules_rev 6 wrote, with `presence` still in the save.
#[given(expr = "an old save from rules_rev 6 with seat presence in it")]
fn an_old_save_from_rules_rev_6(world: &mut World) {
    let pack = crate::support::temp_pack("rules-rev-old-save");
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read save: {err}"));
    let old = text
        .replacen("rules_rev: 7\n", "rules_rev: 6\n", 1)
        .replacen(
            "pause:\n",
            "presence:\n  humans: 1\n  actors_present:\n    - pc_rowan\npause:\n",
            1,
        );
    assert!(
        old.contains("rules_rev: 6\n") && old.contains("\npresence:\n"),
        "{text}"
    );
    fs::write(&path, old).unwrap_or_else(|err| panic!("write save: {err}"));
    world.pack = Some(pack);
}

/// A current save whose journal still holds a `presence-NNNN.json` from rules_rev 6.
#[given(expr = "a current save with a stray presence-0001.json in its journal")]
fn a_current_save_with_a_stray_presence_record(world: &mut World) {
    let pack = crate::support::temp_pack("rules-rev-presence-record");
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read save: {err}"));
    assert!(text.contains("rules_rev: 7\n"), "{text}");
    let journal = pack.join("state/journal");
    fs::create_dir_all(&journal).unwrap_or_else(|err| panic!("mkdir journal: {err}"));
    fs::write(journal.join("base.yaml"), &text).unwrap_or_else(|err| panic!("write base: {err}"));
    let live = text.replacen("state_rev: 0\n", "state_rev: 1\n", 1);
    fs::write(&path, &live).unwrap_or_else(|err| panic!("write save: {err}"));
    let hash = spookery_kernel::api::journal::hash_now(live.as_bytes());
    fs::write(
        journal.join("presence-0001.json"),
        format!(
            "{{\"kind\":\"presence\",\"state_rev_before\":0,\"state_rev_after\":1,\"humans\":1,\"actors_present\":[\"pc_rowan\"],\"now_hash\":\"{hash}\"}}\n"
        ),
    )
    .unwrap_or_else(|err| panic!("write presence: {err}"));
    world.pack = Some(pack);
}

#[when(expr = "spook run or spook verify opens it")]
fn spook_run_or_spook_verify_opens_it(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap();
    let run = crate::support::spook(&["run", arg, "--addr", "127.0.0.1:0"]);
    let verify = crate::support::spook(&["verify", arg]);
    let run_err = String::from_utf8_lossy(&run.stderr);
    let verify_err = String::from_utf8_lossy(&verify.stderr);
    world.code = if run.status.code() == Some(1) && verify.status.code() == Some(1) {
        Some(1)
    } else {
        None
    };
    world.stdout = format!(
        "run:\n{}\nverify:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&verify.stdout)
    );
    world.stderr = format!("run:\n{run_err}\nverify:\n{verify_err}");
}

#[then(expr = "it refuses with RULES_REV and says to start a fresh save")]
fn it_refuses_with_rules_rev_and_says_to_start_a_fresh_save(world: &mut World) {
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    let rest = world
        .stderr
        .strip_prefix("run:\n")
        .unwrap_or_else(|| panic!("missing run stderr\n{}", world.stderr));
    let (run_err, verify_err) = rest
        .split_once("\nverify:\n")
        .unwrap_or_else(|| panic!("missing verify stderr\n{}", world.stderr));
    for (command, err) in [("spook run", run_err), ("spook verify", verify_err)] {
        assert!(
            err.contains("RULES_REV"),
            "{command} did not refuse with RULES_REV\n{err}"
        );
        assert!(
            err.contains("start a fresh save"),
            "{command} did not say to start a fresh save\n{err}"
        );
    }
}

#[given(expr = "logged effects that differ from the re-run")]
fn logged_effects_that_differ_from_the_re_run(world: &mut World) {
    let pack = crate::support::temp_pack("rules-diverge");
    world.pack = Some(pack.clone());
    table::run(world, &["tick", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    let located = spookery_kernel::api::journal::locate(&pack, "tick-0001")
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let text = fs::read_to_string(&located.path).unwrap_or_else(|err| panic!("read tick: {err}"));
    let mut tick: Value = serde_json::from_str(&located.line).unwrap_or_else(|err| {
        panic!("json: {err}\n{text}");
    });
    let effects = tick["effects"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("no effects\n{text}"));
    assert!(!effects.is_empty(), "{text}");
    effects[0]["intent_id"] = Value::String("not-the-replayed-effect".to_string());
    let index = tick["tick_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("no tick_index\n{text}"));
    world.scratch = format!("tick {index}");
    let encoded = serde_json::to_string(&tick).unwrap_or_else(|err| panic!("encode: {err}"));
    let rewritten = text.replacen(&located.line, &encoded, 1);
    assert_ne!(rewritten, text);
    fs::write(&located.path, rewritten).unwrap_or_else(|err| panic!("write tick: {err}"));
}

#[then(expr = "spook verify names the tick and the first differing effect")]
fn spook_verify_names_the_tick_and_the_first_differing_effect(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    assert!(
        world.stderr.contains(world.scratch.trim()),
        "missing {}\n{}",
        world.scratch,
        world.stderr
    );
    assert!(
        world.stderr.contains("effect 0 differed"),
        "{}",
        world.stderr
    );
    assert!(world.stderr.contains("logged "), "{}", world.stderr);
    assert!(world.stderr.contains("replayed "), "{}", world.stderr);
}

const KINDS: &[&str] = &[
    "has_item",
    "meter",
    "flag",
    "location",
    "note",
    "narrate",
    "clock",
    "memory",
    "item",
    "reveal",
    "note_exists",
    "humans",
    "hour",
    "stock",
    "wield",
    "unwield",
    "wielding",
    "at",
    "knows",
    "all",
    "any",
    "not",
];

#[given(expr = "a place rule \"bell\" in docks_shop and a story rule \"bell\"")]
fn a_place_rule_and_a_story_rule(world: &mut World) {
    let pack = temp_pack("rules-ids");
    insert_before_fence(
        &pack.join("world/places/docks_shop.md"),
        "\
rules:
  - id: bell
    when:
      verb: bell
    do: []
",
    );
    write(
        &pack.join("world/items/tonic.md"),
        "\
---
schema: item
id: tonic
name: Tonic
tags: []
rules:
  - id: use
    when:
      verb: use
    do: []
---
A small bottle.
",
    );
    write(
        &pack.join("story/smuggler.yaml"),
        "\
schema: story
id: smuggler
rules:
  - id: bell
    when:
      verb: bell
    do: []
",
    );
    save(&pack, |_| {});
    world.pack = Some(pack);
}

#[then(expr = "spook check --rules lists place:docks_shop\\/bell and story:smuggler\\/bell")]
fn check_rules_lists_both_bells(world: &mut World) {
    run(world, &["check", "--rules"]);
    assert_eq!(world.code, Some(0), "{}\n{}", world.stdout, world.stderr);
    let lines: Vec<&str> = world.stdout.lines().collect();
    let at = |id: &str| {
        lines
            .iter()
            .position(|line| *line == id)
            .unwrap_or_else(|| panic!("missing {id}\n{}", world.stdout))
    };
    let verb = at("verb:wait");
    let place = at("place:docks_shop/bell");
    let item = at("item:tonic/use");
    let cast = at("cast:mira/leave_door_note");
    let story = at("story:smuggler/bell");
    assert!(
        verb < place && place < item && item < cast && cast < story,
        "{lines:?}"
    );
}

#[given(expr = "verb:travel and place:ferry_dock\\/travel")]
fn verb_travel_and_the_ferry_rule(world: &mut World) {
    let pack = temp_pack("rules-ferry");
    write(
        &pack.join("world/places/ferry_dock.md"),
        "\
---
schema: place
id: ferry_dock
display_name: Ferry Dock
facts:
  - id: fact_ferry_waits
    text: The ferry waits.
    hidden: false
rules:
  - id: travel
    when:
      verb: travel
      target: place
    do:
      - flag:
          who: self
          key: took_the_ferry
          value: true
---
The ferry waits at the dock.
",
    );
    let rowan = pack.join("cast/pc-rowan.md");
    let text = fs::read_to_string(&rowan).unwrap();
    let next = text.replace(
        "tick:\n  eligible: false\n",
        "tick:\n  eligible: false\n  flags:\n    - took_the_ferry\n",
    );
    assert_ne!(text, next, "rowan profile was not found");
    fs::write(&rowan, next).unwrap();
    save(&pack, |now| {
        now.places.insert(
            "ferry_dock".to_string(),
            PlaceLive {
                items: Default::default(),
                notes: Vec::new(),
                actors: Vec::new(),
            },
        );
        *now.actors.get_mut("pc_rowan").unwrap().place_mut() = "ferry_dock".to_string();
    });
    world.pack = Some(pack);
}

#[when(expr = "a PC at ferry_dock travels")]
fn a_pc_at_ferry_dock_travels(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let mut opened = open_pack(&pack);
    opened.set_presence(spookery_kernel::api::model::Presence {
        humans: 1,
        actors_present: Vec::new(),
    });
    spookery::host::handle_player_move(&mut opened, "pc_rowan", "travel docks_street", false)
        .unwrap_or_else(|err| panic!("travel {}", err.token()));
}

#[then(expr = "place:ferry_dock\\/travel runs and verb:travel does not")]
fn the_ferry_rule_runs(world: &mut World) {
    let now = read_now(world.pack.as_ref().unwrap());
    let rowan = now.actors.get("pc_rowan").unwrap();
    assert_eq!(rowan.place(), "ferry_dock", "verb:travel moved the PC");
    assert!(
        rowan.flags().iter().any(|flag| flag == "took_the_ferry"),
        "ferry flag missing: {:?}",
        rowan.flags()
    );
}

#[given(expr = "cast:briar\\/restock with hour 6..9 and humans == 0")]
fn briar_restocks_before_the_shop_opens(world: &mut World) {
    let pack = temp_pack("rules-briar");
    write(
        &pack.join("cast/briar.md"),
        "\
---
schema: cast
id: briar
kind: npc
display_name: Briar
facts: []
tick:
  eligible: true
rules:
  - id: restock
    when: habit
    if:
      - hour:
          from: 6
          to: 9
      - humans:
          op: \"==\"
          value: 0
    do:
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: 4
---
Briar tops up the oil before the shop opens.
",
    );
    world.pack = Some(pack);
}

#[when(expr = "an empty tick lands at hour 7 with briar in the shop")]
fn an_empty_tick_at_hour_7(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    save(&pack, |now| {
        now.world_time = 360;
        now.clocks.get_mut("shop_day").unwrap().value = 0;
        let briar = now.actors.get("mira").unwrap().clone();
        now.actors.insert("briar".to_string(), briar);
        *now.actors.get_mut("briar").unwrap().place_mut() = "docks_shop".to_string();
    });
    let mut opened = open_pack(&pack);
    let wall = opened.now().last_tick_at.clone();
    play(&mut opened, RoundCause::Manual, &wall).unwrap_or_else(|err| {
        panic!(
            "empty tick {}: {}",
            err.token(),
            spookery_kernel::api::check_pack(&pack)
                .iter()
                .map(|diag| diag.report_line())
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
}

#[then(expr = "docks_shop lamp_oil increases by 4")]
fn lamp_oil_increases_by_4(world: &mut World) {
    let now = read_now(world.pack.as_ref().unwrap());
    let oil = now
        .places
        .get("docks_shop")
        .and_then(|place| place.items.get("lamp_oil"))
        .copied();
    assert_eq!(oil, Some(8u32), "world_time {}", now.world_time);
}

#[given(expr = "attack targets defense + 10 and briar's defense is 12")]
fn attack_targets_defense(world: &mut World) {
    let pack = copy_pack("sample-d20", "rules-attack");
    append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: attack
    when:
      verb: attack
      target: actor
    roll:
      check: strike
      target:
        meter:
          who: target
          key: defense
        plus: 10
    success:
      - flag:
          who: target
          key: hit
          value: true
    failure:
      - flag:
          who: target
          key: missed
          value: true
"#,
    );
    let briar = pack.join("cast/briar.md");
    let text = fs::read_to_string(&briar).unwrap();
    let next = text.replace(
        "  eligible: true\n",
        "  eligible: true\n  flags:\n    - hit\n    - missed\n",
    );
    assert_ne!(text, next, "briar profile was not found");
    fs::write(&briar, next).unwrap();
    save(&pack, |now| {
        let defense = now
            .actors
            .get_mut("npc_briar")
            .unwrap()
            .meters_mut()
            .get_mut("defense")
            .unwrap();
        defense.base = 12;
        defense.offset = 0;
        now.actors
            .get_mut("npc_briar")
            .unwrap()
            .act_mut()
            .busy_until = Some(u64::MAX);
    });
    world.pack = Some(pack);
}

#[when(expr = "lark attacks with a seeded total of 21")]
fn lark_attacks(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let loaded = read_now(&pack);
    let face = faces(
        &RollKey {
            world_id: loaded.shard_id.clone(),
            round_index: loaded.tick_index + 1,
            rule_id: "attack".to_string(),
            subject_id: "pc_lark".to_string(),
            roll_index: 0,
        },
        1,
        20,
    )
    .unwrap_or_else(|err| panic!("faces {}", err.token()))[0];
    save(&pack, |now| {
        let might = now
            .actors
            .get_mut("pc_lark")
            .unwrap()
            .meters_mut()
            .get_mut("might")
            .unwrap();
        might.base = 21 - i64::from(face);
        might.offset = 0;
    });
    let mut opened = open_pack(&pack);
    let notices =
        match spookery::host::handle_player_move(&mut opened, "pc_lark", "attack npc_briar", false)
        {
            Ok(spookery::host::MoveOutcome::Committed { notices, .. }) => notices,
            Ok(spookery::host::MoveOutcome::Queued(_)) => panic!("attack queued"),
            Err(err) => panic!("attack {}", err.token()),
        };
    let rolled = notices.into_iter().find_map(|notice| match notice {
        RollReport::Rolled { actor_id, out } if actor_id == "pc_lark" => Some(out),
        _ => None,
    });
    let rolled = rolled.unwrap_or_else(|| panic!("Lark did not roll"));
    assert_eq!(rolled.total, 21, "{rolled:?}");
    assert_eq!(rolled.target, 22, "{rolled:?}");
    world.scratch = rolled.outcome.as_str().to_string();
}

#[then(expr = "the outcome is a miss")]
fn the_outcome_is_a_miss(world: &mut World) {
    assert_eq!(world.scratch, "miss");
    let now = read_now(world.pack.as_ref().unwrap());
    let flags = now.actors.get("npc_briar").unwrap().flags();
    assert!(flags.iter().any(|flag| flag == "missed"), "{flags:?}");
    assert!(!flags.iter().any(|flag| flag == "hit"), "{flags:?}");
}

#[given(expr = "a pack with rules\\/moves.yaml")]
fn a_pack_with_moves_yaml(world: &mut World) {
    let pack = temp_pack("rules-moves");
    fs::write(pack.join("rules/moves.yaml"), "schema: moves\n").unwrap();
    world.pack = Some(pack);
}

#[then(expr = "spook check says moves.yaml was replaced by rules\\/verbs.yaml")]
fn check_says_moves_was_replaced(world: &mut World) {
    run(world, &["check"]);
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    assert!(
        world
            .stderr
            .contains("moves.yaml was replaced by rules/verbs.yaml"),
        "{}",
        world.stderr
    );
}

#[given(expr = "a verb file where the word bogus appears in earlier text")]
fn bogus_appears_in_earlier_text(world: &mut World) {
    let pack = temp_pack("rules-bogus");
    append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: tale
    when:
      verb: tale
    do:
      - narrate:
          to: self
          text: the bogus rumor
"#,
    );
    world.pack = Some(pack);
}

#[when(expr = "an effect key later in that file is bogus")]
fn an_effect_key_is_bogus(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    append(&pack.join("rules/verbs.yaml"), "      - bogus: true\n");
    save(&pack, |_| {});
}

#[then(expr = "spook check reports that key at its own line and column")]
fn check_reports_the_key(world: &mut World) {
    let text = fs::read_to_string(world.pack.as_ref().unwrap().join("rules/verbs.yaml")).unwrap();
    let (line, column) = key_spot(&text, "bogus:");
    let rumor = text
        .lines()
        .position(|row| row.contains("the bogus rumor"))
        .unwrap()
        + 1;
    assert_ne!(line as usize, rumor);
    run(world, &["check", "--json"]);
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    let items: Vec<Value> = serde_json::from_str(world.stdout.trim())
        .unwrap_or_else(|err| panic!("{err}: {}\n{}", world.stdout, world.stderr));
    let hit = items
        .iter()
        .find(|item| {
            item["message"]
                .as_str()
                .is_some_and(|message| message.contains("unknown field 'bogus'"))
        })
        .unwrap_or_else(|| panic!("{}", world.stdout));
    assert_eq!(hit["file"], "rules/verbs.yaml");
    assert_eq!(hit["line"], line);
    assert_eq!(hit["column"], column, "{hit}");
    assert_ne!(hit["line"].as_u64(), Some(rumor as u64));
}

#[given(expr = "a verb whose if dash holds two conditions")]
fn two_conditions_under_one_dash(world: &mut World) {
    let pack = temp_pack("rules-two");
    append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: both
    when:
      verb: both
    if:
      - has_item:
          who: self
          id: knife
        flag:
          who: self
          key: marked
          is: true
    do: []
"#,
    );
    save(&pack, |_| {});
    world.pack = Some(pack);
}

#[then(expr = "spook check reports two conditions under one dash")]
fn check_reports_two_conditions(world: &mut World) {
    run(world, &["check"]);
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    assert!(
        world.stderr.contains("two conditions under one dash"),
        "{}",
        world.stderr
    );
}

#[then(expr = "no effect or condition kind is matched by string in canon\\/")]
fn canon_does_not_match_kinds_by_string(world: &mut World) {
    let _ = world;
    let root = repo().join("crates/kernel/canon");
    let mut hits = Vec::new();
    for path in rust_files(&root) {
        let text = fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            if kind_compared(line) {
                hits.push(format!(
                    "{}:{}: {line}",
                    path.strip_prefix(repo()).unwrap().display(),
                    index + 1
                ));
            }
        }
    }
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}

#[then(expr = "the host sources contain no audience or item checks")]
fn the_host_has_no_audience_or_item_checks(world: &mut World) {
    let _ = world;
    let root = repo().join("crates/spook/src/host");
    let banned = ["audience", "require_item", "has_item", "inventory"];
    let mut hits = Vec::new();
    for path in rust_files(&root) {
        let text = fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            for word in banned {
                if line.contains(word) {
                    hits.push(format!(
                        "{}:{}: {line}",
                        path.strip_prefix(repo()).unwrap().display(),
                        index + 1
                    ));
                }
            }
        }
    }
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}

#[given(expr = "steady costs 5 grit and the sitter has 1")]
fn steady_costs_five_grit(world: &mut World) {
    let pack = temp_pack("rules-cost");
    append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: steady
    when:
      verb: steady
    cost:
      meter:
        grit: 5
    do: []
"#,
    );
    save(&pack, |now| {
        let grit = now
            .actors
            .get_mut("pc_rowan")
            .unwrap()
            .meters_mut()
            .get_mut("grit")
            .unwrap();
        grit.base = 1;
        grit.offset = 0;
    });
    world.pack = Some(pack);
}

#[when(expr = "the sitter plays steady")]
fn the_sitter_plays_steady(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let mut opened = open_pack(&pack);
    world.scratch =
        match spookery::host::handle_player_move(&mut opened, "pc_rowan", "steady", false) {
            Err(err) => format!("{} {}", err.token(), err.missing().unwrap_or("")),
            Ok(_) => "queued".to_string(),
        };
}

#[then(expr = "the refusal is CANT_AFFORD naming grit and world_time stays 1080")]
fn the_refusal_is_cant_afford(world: &mut World) {
    assert_eq!(
        world.scratch,
        format!("{} grit", Reason::CantAfford.token())
    );
    let now = read_now(world.pack.as_ref().unwrap());
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.tick_index, 0);
}

#[given(
    expr = "a habit cast:mira\\/leave_door_note and a place rule place:docks_street\\/travel while the PC stays in the shop"
)]
fn a_far_place_rule_while_the_pc_stays(world: &mut World) {
    let pack = temp_pack("rules-full-id");
    insert_before_fence(
        &pack.join("world/places/docks_street.md"),
        "\
rules:
  - id: travel
    when:
      verb: travel
      target: place
    do:
      - flag:
          who: self
          key: took_the_street
          value: true
",
    );
    save(&pack, |_| {});
    let now = read_now(&pack);
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "docks_shop");
    world.pack = Some(pack);
}

#[when(expr = "a PC is gated on those full ids")]
fn a_pc_is_gated_on_those_full_ids(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let opened = open_pack(&pack);
    let mut codes = Vec::new();
    for move_name in ["cast:mira/leave_door_note", "place:docks_street/travel"] {
        let intent = pc_probe(move_name);
        let resolved = gate::resolve(opened.canon(), opened.now(), &intent);
        let gated = gate::gate(opened.canon(), opened.now(), &intent);
        match (resolved, gated) {
            (Err(reason), Err(miss))
                if reason == Reason::NotInSpec && miss.reason() == Reason::NotInSpec =>
            {
                codes.push(reason.token());
            }
            (resolved, gated) => panic!("{move_name}: resolved={resolved:?} gated={gated:?}"),
        }
    }
    world.scratch = codes.join(" ");
}

#[then(expr = "both are refused with NOT_IN_SPEC and the save is unchanged")]
fn both_full_ids_are_refused(world: &mut World) {
    assert_eq!(
        world.scratch,
        format!(
            "{} {}",
            Reason::NotInSpec.token(),
            Reason::NotInSpec.token()
        )
    );
    let now = read_now(world.pack.as_ref().unwrap());
    let rowan = now.actors.get("pc_rowan").unwrap();
    assert_eq!(rowan.place(), "docks_shop");
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.tick_index, 0);
    assert!(
        now.messages
            .iter()
            .all(|message| message.id != "note_mira_door"),
        "the habit note was stored"
    );
    assert!(
        !rowan.flags().iter().any(|flag| flag == "took_the_street"),
        "the far place rule ran"
    );
}

#[given(expr = "use on tonic targets an actor and the shared use takes no target")]
fn use_on_tonic_names_the_next_token(world: &mut World) {
    let pack = temp_pack("rules-owned");
    write(
        &pack.join("world/items/tonic.md"),
        "\
---
schema: item
id: tonic
name: Tonic
tags: []
rules:
  - id: use
    when:
      verb: use
      target: item
    do: []
---
A small bottle.
",
    );
    let verbs = pack.join("rules/verbs.yaml");
    let text = fs::read_to_string(&verbs).unwrap();
    let from = "  - id: use
    when:
      verb: use
      target: item
    if:
      - has_item:
          who: self
          id: target
    do: []
";
    let to = "  - id: use
    when:
      verb: use
    do: []
";
    assert!(
        text.contains(from),
        "shared use was not the item-target fallback"
    );
    fs::write(&verbs, text.replacen(from, to, 1)).unwrap();
    save(&pack, |_| {});
    world.pack = Some(pack);
}

#[when(expr = "the sitter lines up use tonic")]
fn the_sitter_lines_up_use_tonic(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let mut opened = open_pack(&pack);
    world.scratch =
        match spookery::host::handle_player_move(&mut opened, "pc_rowan", "use tonic", true) {
            Ok(spookery::host::MoveOutcome::Queued(note)) => {
                let queued = opened
                    .queued_intents()
                    .iter()
                    .find(|intent| intent.id == note.id)
                    .unwrap();
                format!("{:?} {}", queued.target, queued.move_name)
            }
            Ok(spookery::host::MoveOutcome::Committed { .. }) => "committed".to_string(),
            Err(err) => err.token().to_string(),
        };
}

#[then(expr = "the queued target is that actor and the save is unchanged")]
fn the_queued_target_is_that_actor(world: &mut World) {
    assert_eq!(world.scratch, "Actor(\"tonic\") use");
    let now = read_now(world.pack.as_ref().unwrap());
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "docks_shop");
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.tick_index, 0);
}

#[given(expr = "a verb with a time cost and a resist roll")]
fn a_verb_with_time_and_resist(world: &mut World) {
    let pack = temp_pack("rules-unused");
    append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: linger
    when:
      verb: linger
    cost:
      time: 10m
    do:
      - meter:
          who: self
          key: grit
          add: 1
          resist:
            who: self
            check: grit
            target: 2
"#,
    );
    save(&pack, |_| {});
    world.pack = Some(pack);
}

#[then(expr = "spook check is quiet about the time and the resist")]
fn check_is_quiet_about_time_and_resist(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    open_pack(&pack);
    run(world, &["check"]);
    assert_eq!(world.code, Some(0), "{}\n{}", world.stdout, world.stderr);
    assert!(!world.stderr.contains("cost time"), "{}", world.stderr);
    assert!(!world.stderr.contains("resist"), "{}", world.stderr);
}

const BARE_SAME: &str = "
  - id: shrug
    when:
      verb: shrug
    do:
      - meter:
          who: self
          key: grit
          add: 1
          resist: same
";

const SAME_AFTER_ROLL: &str = "
  - id: shrug
    when:
      verb: shrug
    do:
      - meter:
          who: self
          key: grit
          add: -1
          resist:
            who: self
            check: grit
            target: 2
            resisted: []
      - meter:
          who: self
          key: grit
          add: 1
          resist: same
";

#[given(expr = "a verb whose only resist is same")]
fn a_verb_whose_only_resist_is_same(world: &mut World) {
    let pack = temp_pack("rules-bare-same");
    append(&pack.join("rules/verbs.yaml"), BARE_SAME);
    save(&pack, |_| {});
    world.pack = Some(pack);
}

#[then(expr = "spook check reports resist: same with no earlier roll")]
fn check_reports_bare_resist_same(world: &mut World) {
    run(world, &["check"]);
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    assert!(
        world.stderr.contains("resist: same before any resist roll"),
        "{}",
        world.stderr
    );
    assert!(world.stderr.contains("SCHEMA"), "{}", world.stderr);
}

#[given(expr = "a verb whose resist same follows a roll")]
fn a_verb_whose_resist_same_follows_a_roll(world: &mut World) {
    let pack = temp_pack("rules-same-after");
    append(&pack.join("rules/verbs.yaml"), SAME_AFTER_ROLL);
    save(&pack, |_| {});
    world.pack = Some(pack);
}

#[then(expr = "spook check does not report a bare resist same")]
fn check_accepts_resist_same_after_a_roll(world: &mut World) {
    run(world, &["check"]);
    assert_eq!(world.code, Some(0), "{}\n{}", world.stdout, world.stderr);
    assert!(
        !world.stderr.contains("resist: same before any resist roll"),
        "{}",
        world.stderr
    );
}

fn pc_probe(move_name: &str) -> Intent {
    Intent {
        id: "probe".to_string(),
        tick_index: 0,
        actor_id: "pc_rowan".to_string(),
        actor_kind: ActorKind::Player,
        source: Source::Player,
        move_name: move_name.to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn run(world: &mut World, args: &[&str]) {
    let pack = crate::support::pack_arg(world.pack.as_ref().unwrap());
    let mut full = Vec::with_capacity(args.len() + 1);
    full.extend_from_slice(args);
    full.push(pack.as_str());
    let output = spook(&full);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

fn open_pack(pack: &Path) -> spookery_kernel::api::World {
    spookery_round::resume(pack).unwrap_or_else(|err| {
        panic!(
            "open {}: {}",
            err.token(),
            spookery_kernel::api::check_pack(pack)
                .iter()
                .map(|diag| diag.report_line())
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

fn save(pack: &Path, edit: impl FnOnce(&mut Now)) {
    let path = pack.join("state/save.yaml");
    let mut now = read_now(pack);
    edit(&mut now);
    now.pack_rev = spookery_kernel::api::canon::pack_rev(pack).unwrap();
    fs::write(&path, emit_save(&now).unwrap()).unwrap();
}

fn read_now(pack: &Path) -> Now {
    let report =
        spookery_round::from_journal(pack, None, None).unwrap_or_else(|err| panic!("{err}"));
    let text = std::str::from_utf8(&report.now_yaml).unwrap();
    load_save(text).unwrap()
}

fn append(path: &Path, extra: &str) {
    let mut text = fs::read_to_string(path).unwrap();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(extra);
    fs::write(path, text).unwrap();
}

fn insert_before_fence(path: &Path, block: &str) {
    let text = fs::read_to_string(path).unwrap();
    let index = text
        .find("\n---\n")
        .unwrap_or_else(|| panic!("no fence in {}", path.display()));
    let mut next = text[..index].to_string();
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(block);
    if !block.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&text[index + 1..]);
    fs::write(path, next).unwrap();
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn key_spot(text: &str, key: &str) -> (u64, u64) {
    for (index, line) in text.lines().enumerate() {
        if let Some(column) = line.find(key) {
            return ((index + 1) as u64, (column + 1) as u64);
        }
    }
    panic!("missing {key}");
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out
}

fn kind_compared(line: &str) -> bool {
    KINDS.iter().any(|kind| {
        line.contains(&format!("== \"{kind}\""))
            || line.contains(&format!("!= \"{kind}\""))
            || line.contains(&format!("\"{kind}\" =="))
            || line.contains(&format!("\"{kind}\" !="))
    })
}
