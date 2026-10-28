use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery::host::{idle_clock::wall_now, MoveOutcome};
use spookery_engine::handle_player_move;
use spookery_engine::testing::{faces, PlaceLive, RollKey};
use spookery_engine::MoveReport;
use spookery_engine::Reason;

use super::table;
use crate::support::{append, copy_pack, edit_save, now_of, spook, temp_pack};
use crate::World;
#[given(expr = "a save with rules_rev 8 and an engine at 9")]
fn a_save_with_rules_rev_8_and_an_engine_at_9(world: &mut World) {
    assert_eq!(
        spookery_engine::testing::RULES_REV,
        9,
        "the engine rules version is 9"
    );
    let pack = crate::support::temp_pack("rules-rev");
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read save: {err}"));
    let stamped = text.replacen("rules_rev: 9\n", "rules_rev: 8\n", 1);
    assert_ne!(stamped, text, "save did not carry rules_rev 9\n{text}");
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
        .replacen("rules_rev: 9\n", "rules_rev: 6\n", 1)
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
    assert!(text.contains("rules_rev: 9\n"), "{text}");
    let journal = pack.join("state/journal");
    fs::create_dir_all(&journal).unwrap_or_else(|err| panic!("mkdir journal: {err}"));
    fs::write(journal.join("base.yaml"), &text).unwrap_or_else(|err| panic!("write base: {err}"));
    let live = text.replacen("state_rev: 0\n", "state_rev: 1\n", 1);
    fs::write(&path, &live).unwrap_or_else(|err| panic!("write save: {err}"));
    let hash = spookery_engine::testing::journal::hash_now(live.as_bytes());
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
    let located = spookery_engine::testing::journal::locate(&pack, "round-0001")
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let text = fs::read_to_string(&located.path).unwrap_or_else(|err| panic!("read tick: {err}"));
    let line = text
        .lines()
        .find(|l| serde_json::from_str::<Value>(l).unwrap()["round_index"] == 1)
        .unwrap();
    let mut round: Value = serde_json::from_str(line).unwrap();
    let effects = round["effects"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("no effects\n{text}"));
    assert!(!effects.is_empty(), "{text}");
    effects[0]["intent_id"] = Value::String("not-the-replayed-effect".to_string());
    let index = round["round_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("no round_index\n{text}"));
    world.scratch = format!("round {index}");
    let encoded = serde_json::to_string(&round).unwrap_or_else(|err| panic!("encode: {err}"));
    let rewritten = text.replacen(line, &encoded, 1);
    assert_ne!(rewritten, text);
    fs::write(&located.path, rewritten).unwrap_or_else(|err| panic!("write tick: {err}"));
}

#[then(expr = "spook verify names the round and the first differing effect")]
fn spook_verify_names_the_round_and_the_first_differing_effect(world: &mut World) {
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
    edit_save(&pack, |_| {});
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
    edit_save(&pack, |now| {
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
    opened.set_presence(spookery_engine::testing::Presence {
        humans: 1,
        actors_present: Vec::new(),
    });
    let wall = wall_now();
    let played = handle_player_move(
        &mut opened,
        "pc_rowan",
        "travel",
        Some("ferry_dock"),
        false,
        &wall,
    );
    played.unwrap_or_else(|err| panic!("travel {}", err.token()));
}

#[then(expr = "place:ferry_dock\\/travel runs and verb:travel does not")]
fn the_ferry_rule_runs(world: &mut World) {
    let now = now_of(world.pack.as_ref().unwrap());
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

#[when(expr = "an empty round lands at hour 7 with briar in the shop")]
fn an_empty_round_at_hour_7(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    edit_save(&pack, |now| {
        now.world_time = 360;
        now.clocks.get_mut("shop_day").unwrap().value = 0;
        let briar = now.actors.get("mira").unwrap().clone();
        now.actors.insert("briar".to_string(), briar);
        *now.actors.get_mut("briar").unwrap().place_mut() = "docks_shop".to_string();
    });
    let mut opened = open_pack(&pack);
    let wall = opened.now().last_round_at.clone();
    opened.play(&wall).unwrap_or_else(|err| {
        panic!(
            "empty round {}: {}",
            err.token(),
            spookery_engine::check_pack(&pack)
                .iter()
                .map(|diag| diag.report_line())
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
}

#[then(expr = "docks_shop lamp_oil increases by 4")]
fn lamp_oil_increases_by_4(world: &mut World) {
    let now = now_of(world.pack.as_ref().unwrap());
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
    edit_save(&pack, |now| {
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
    let loaded = now_of(&pack);
    let face = faces(
        &RollKey {
            world_id: loaded.shard_id.clone(),
            round_index: loaded.round_index + 1,
            rule_id: "verb:attack".to_string(),
            subject_id: "pc_lark".to_string(),
            roll_index: 0,
        },
        1,
        20,
    )
    .unwrap_or_else(|err| panic!("faces {}", err.token()))[0];
    edit_save(&pack, |now| {
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
    let wall = wall_now();
    let reports = match handle_player_move(
        &mut opened,
        "pc_lark",
        "attack",
        Some("npc_briar"),
        false,
        &wall,
    ) {
        Ok(MoveOutcome::Committed { outputs, .. }) => outputs.into_iter().next().unwrap().reports,
        Ok(MoveOutcome::Queued(_)) => panic!("attack queued"),
        Ok(MoveOutcome::Busy { .. }) => panic!("attack was busy"),
        Err(err) => panic!("attack {}", err.token()),
    };
    let rolled = reports.into_iter().find_map(|report| match report {
        MoveReport::Rolled { actor_id, out } if actor_id == "pc_lark" => Some(out),
        _ => None,
    });
    let rolled = rolled.unwrap_or_else(|| panic!("Lark did not roll"));
    assert_eq!(rolled.total, 21, "{rolled:?}");
    assert_eq!(rolled.target, 22, "{rolled:?}");
    world.scratch = match rolled.outcome {
        spookery_engine::Outcome::Success => "success",
        spookery_engine::Outcome::Partial => "partial",
        spookery_engine::Outcome::Miss => "miss",
    }
    .to_string();
}

#[then(expr = "the outcome is a miss")]
fn the_outcome_is_a_miss(world: &mut World) {
    assert_eq!(world.scratch, "miss");
    let now = now_of(world.pack.as_ref().unwrap());
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
    edit_save(&pack, |_| {});
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
    edit_save(&pack, |_| {});
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
    edit_save(&pack, |now| {
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
    let wall = wall_now();
    let played = handle_player_move(&mut opened, "pc_rowan", "steady", None, false, &wall);
    world.scratch = match played {
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
    let now = now_of(world.pack.as_ref().unwrap());
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.round_index, 0);
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
    edit_save(&pack, |_| {});
    let now = now_of(&pack);
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "docks_shop");
    world.pack = Some(pack);
}

#[when(expr = "a PC is gated on those full ids")]
fn a_pc_is_gated_on_those_full_ids(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let mut opened = open_pack(&pack);
    let mut codes = Vec::new();
    for move_name in ["cast:mira/leave_door_note", "place:docks_street/travel"] {
        match opened.queue_player_move("pc_rowan", move_name, None) {
            Err(miss) if miss.reason() == Reason::NotInSpec => {
                codes.push(miss.token());
            }
            other => panic!("{move_name}: {other:?}"),
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
    let now = now_of(world.pack.as_ref().unwrap());
    let rowan = now.actors.get("pc_rowan").unwrap();
    assert_eq!(rowan.place(), "docks_shop");
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.round_index, 0);
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
    append(
        &pack.join("rules/verbs.yaml"),
        "
  - id: use
    when:
      verb: use
    do: []
",
    );
    edit_save(&pack, |now| {
        let rowan = now.actors.get_mut("pc_rowan").unwrap();
        rowan.inventory_mut().insert("tonic".to_string(), 1);
    });
    world.pack = Some(pack);
}

#[when(expr = "the sitter lines up use tonic")]
fn the_sitter_lines_up_use_tonic(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let mut opened = open_pack(&pack);
    let wall = wall_now();
    let played = handle_player_move(&mut opened, "pc_rowan", "use", Some("tonic"), true, &wall);
    world.scratch = match played {
        Ok(MoveOutcome::Busy { .. }) => panic!("use tonic was busy"),
        Ok(MoveOutcome::Queued(queued)) => {
            let intent = opened
                .queued_intents()
                .iter()
                .find(|intent| intent.id == queued.id)
                .unwrap();
            format!("{:?} {}", intent.target, intent.move_name)
        }
        Ok(MoveOutcome::Committed { .. }) => "committed".to_string(),
        Err(err) => err.token().to_string(),
    };
}

#[then(expr = "the queued target is that actor and the save is unchanged")]
fn the_queued_target_is_that_actor(world: &mut World) {
    assert_eq!(world.scratch, "Actor(\"tonic\") use");
    let now = now_of(world.pack.as_ref().unwrap());
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "docks_shop");
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.round_index, 0);
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
    edit_save(&pack, |_| {});
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
    edit_save(&pack, |_| {});
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
    edit_save(&pack, |_| {});
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

fn open_pack(pack: &Path) -> spookery_engine::World {
    spookery_engine::World::resume(pack).unwrap_or_else(|err| {
        panic!(
            "open {}: {}",
            err.token(),
            spookery_engine::check_pack(pack)
                .iter()
                .map(|diag| diag.report_line())
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
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
