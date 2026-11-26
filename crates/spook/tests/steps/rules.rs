use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::roll;

use crate::support::{edit_as_saved, edit_meter, insert_before_fence, put_items, put_meter};
use spookery_engine::{
    self as engine, component, entity, ActorId, GlobalId, Meter, Note, Presence, Reason,
};

use super::common::flags_of;
use super::host;
use crate::support::{append, copy_pack, edit_save, place_of, run_spook, stamp_pack_rev, world_of};
use crate::World;
/// The first word of `line`, and the rest of it when there is any.
pub(crate) fn split_line(line: &str) -> (&str, Option<&str>) {
    line.split_once(' ')
        .map_or((line, None), |(head, rest)| (head, Some(rest)))
}

#[given(expr = "a save from older rules")]
fn a_save_from_older_rules(world: &mut World) {
    // repeats module: mirrors its step text
    let rev = spookery_engine::testing::RULES_REV;
    let pack = crate::support::copy_pack("docks", "rules-rev");
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read save: {err}"));
    let stamped = text.replacen(
        &format!("rules_rev: {rev}\n"),
        &format!("rules_rev: {}\n", rev - 1),
        1,
    );
    assert_ne!(stamped, text, "save did not carry rules_rev {rev}\n{text}");
    fs::write(&path, stamped).unwrap_or_else(|err| panic!("write save: {err}"));
    world.pack = Some(pack);
}

#[when(expr = "spook run or spook verify opens it")]
async fn spook_run_or_spook_verify_opens_it(world: &mut World) {
    let pack = host::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap();
    let (run_status, run_stdout, run_stderr) =
        run_spook(&["run", arg, "--addr", "127.0.0.1:0"]).await;
    let (verify_status, verify_stdout, verify_stderr) = run_spook(&["verify", arg]).await;
    world.code = if run_status.code() == Some(1) && verify_status.code() == Some(1) {
        Some(1)
    } else {
        None
    };
    world.stdout = format!("run:\n{run_stdout}\nverify:\n{verify_stdout}");
    world.stderr = format!("run:\n{run_stderr}\nverify:\n{verify_stderr}");
}

#[then(expr = "it refuses with RULES_REV_MISMATCH and says to start a fresh save")]
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
            err.contains("RULES_REV_MISMATCH"),
            "{command} did not refuse with RULES_REV_MISMATCH\n{err}"
        );
        assert!(
            err.contains("start a fresh save"),
            "{command} did not say to start a fresh save\n{err}"
        );
    }
}

#[given(expr = "logged effects that differ from the re-run")]
async fn logged_effects_that_differ_from_the_re_run(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "rules-diverge");
    world.pack = Some(pack.clone());
    host::run(world, &["round", pack.to_str().unwrap()]).await;
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    let path = spookery_engine::testing::journal::locate(&pack, "round-0001")
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read round: {err}"));
    let line = text
        .lines()
        .find(|l| serde_json::from_str::<Value>(l).unwrap()["round_index"] == 1)
        .unwrap();
    let mut round: Value = serde_json::from_str(line).unwrap();
    let effects = round["effects"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("no effects\n{text}"));
    assert!(!effects.is_empty(), "{text}");
    effects[0]["intent"] = Value::String("not-the-replayed-effect".to_string());
    let index = round["round_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("no round_index\n{text}"));
    world.scratch = format!("round {index}");
    let encoded = serde_json::to_string(&round).unwrap_or_else(|err| panic!("encode: {err}"));
    let rewritten = text.replacen(line, &encoded, 1);
    assert_ne!(rewritten, text);
    fs::write(&path, rewritten).unwrap_or_else(|err| panic!("write round: {err}"));
}

#[then(expr = "spook verify names the round and the first differing effect")]
async fn spook_verify_names_the_round_and_the_first_differing_effect(world: &mut World) {
    let pack = host::pack(world).to_path_buf();
    host::run(world, &["verify", pack.to_str().unwrap()]).await;
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
    let pack = copy_pack("docks", "rules-ids");
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
async fn check_rules_lists_both_bells(world: &mut World) {
    run(world, &["check", "--rules"]).await;
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
    let pack = copy_pack("docks", "rules-ferry");
    write(
        &pack.join("world/places/ferry_dock.md"),
        "\
---
schema: place
id: ferry_dock
name: Ferry Dock
rules:
  - id: travel
    when:
      verb: travel
      target: place
    do:
      - flag:
          of: self
          id: took_the_ferry
          value: true
---
The ferry waits at the dock.
",
    );
    edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("ferry_dock")
            .unwrap();
    });
    world.pack = Some(pack);
}

#[when(expr = "a PC at ferry_dock travels")]
fn a_pc_at_ferry_dock_travels(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let mut opened = open(&pack);
    let one_human = Presence {
        players: 1,
        actors_present: Vec::new(),
    };
    opened
        .queue_player_verb(&one_human, &"rowan".into(), "travel", Some("ferry_dock"))
        .unwrap_or_else(|err| panic!("travel {}", err.code.as_str()));
    let wall = opened.world().last_round_at.clone();
    opened
        .commit_then_fast_forward(&one_human, &wall)
        .unwrap_or_else(|err| panic!("travel {err}"));
}

#[then(expr = "place:ferry_dock\\/travel runs and verb:travel does not")]
fn the_ferry_rule_runs(world: &mut World) {
    let now = world_of(world.pack.as_ref().unwrap());
    assert_eq!(
        place_of(&now, "rowan"),
        "ferry_dock",
        "verb:travel moved the PC"
    );
    let flags = flags_of(&now, "rowan");
    assert!(
        flags.iter().any(|flag| flag == "took_the_ferry"),
        "ferry flag missing: {flags:?}"
    );
}

#[given(expr = "cast:briar\\/restock with cycle day 360..540 and players == 0")]
fn briar_restocks_before_the_shop_opens(world: &mut World) {
    let pack = copy_pack("docks", "rules-briar");
    write_briar(&pack, "day", 360, 540);
    world.pack = Some(pack);
}

/// Set cycle `cycle_id` to `length` ticks, adding it beside the docks `day` when it is new.
#[given(expr = "rules\\/budget.yaml sets cycle {word} to {int}")]
fn budget_sets_cycle(world: &mut World, cycle_id: String, length: u64) {
    let pack = copy_pack("docks", "rules-cycle");
    let budget = pack.join("rules/budget.yaml");
    let (old, new) = match cycle_id.as_str() {
        "day" => ("  day: 1440\n".to_string(), format!("  day: {length}\n")),
        _ => (
            "cycles:\n".to_string(),
            format!("cycles:\n  {cycle_id}: {length}\n"),
        ),
    };
    super::common::replace_in_file(&budget, &old, &new);
    world.pack = Some(pack);
}

#[given(expr = "briar restocks on cycle {word} from {int} to {int}")]
fn briar_restocks_on_cycle(world: &mut World, cycle_id: String, from: u64, to: u64) {
    write_briar(host::pack(world), &cycle_id, from, to);
}

fn write_briar(pack: &Path, cycle: &str, from: u64, to: u64) {
    write(
        &pack.join("cast/briar.md"),
        &format!(
            "\
---
schema: cast
id: briar
control_type: npc
name: Briar
idle:
  eligible: true
rules:
  - id: restock
    when: habit
    if:
      - cycle:
          id: {cycle}
          from: {from}
          to: {to}
      - players:
          op: \"==\"
          value: 0
    do:
      - item:
          of: place:docks_shop
          id: lamp_oil
          add: 4
---
Briar tops up the oil before the shop opens.
"
        ),
    );
}

/// The round starts one idle round's ticks before `tick`. Mira has used her idle acts, so
/// only briar can restock.
#[when(expr = "an empty round lands at tick {int} with briar in the shop")]
fn an_empty_round_lands_at(world: &mut World, tick: u64) {
    let pack = world.pack.clone().unwrap();
    edit_save(&pack, |now| {
        now.world_time = tick - 60;
        edit_meter(now, "mira", "shop_day", |shop_day| shop_day.base = 0);
        let mira_place = place_of(now, "mira");
        let (mira, briar) = (&"mira".into(), &"briar".into());
        let mut actors = now.actors_mut();
        actors.copy_actor(mira, briar, &mira_place).unwrap();
        actors
            .get_mut::<Meter>(briar)
            .set("shop_day", None)
            .unwrap();
        actors
            .get_mut::<component::Place>(briar)
            .set("docks_shop")
            .unwrap();
        edit_as_saved(now, |save| {
            save["actors"]["acts"]["mira"] = act_row_with_idle_acts(3)
        });
    });
    play_an_empty_round(&pack);
}

/// The save row of an actor who has not acted but has used `count` idle acts this window.
fn act_row_with_idle_acts(count: u32) -> serde_json::Value {
    serde_json::json!({
        "last_act_round": 0,
        "idle_acts_this_window": count,
        "live_acts_this_window": 0,
    })
}

fn play_an_empty_round(pack: &Path) {
    let mut opened = open(pack);
    let wall = opened.world().last_round_at.clone();
    opened
        .play(&Presence::default(), &wall)
        .unwrap_or_else(|err| {
            panic!(
                "empty round {}: {}",
                err.as_str(),
                spookery_engine::check_pack(pack)
                    .iter()
                    .map(|diagnostic| diagnostic.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
}

#[then(expr = "docks_shop lamp_oil increases by 4")]
fn lamp_oil_increases_by_4(world: &mut World) {
    let now = world_of(world.pack.as_ref().unwrap());
    let oil = now
        .places()
        .get::<component::Item>(&"docks_shop".into())
        .get("lamp_oil")
        .copied();
    assert_eq!(oil, Some(8u32), "world_time {}", now.world_time);
}

#[given(
    expr = "idle_act_window_ticks is {int} and every actor has used its idle acts at tick {int}"
)]
fn every_actor_used_its_idle_acts(world: &mut World, window: u64, tick: u64) {
    let pack = copy_pack("docks", "rules-idle-window");
    super::common::replace_in_file(
        &pack.join("rules/budget.yaml"),
        "idle_act_window_ticks: 1440\n",
        &format!("idle_act_window_ticks: {window}\n"),
    );
    edit_save(&pack, |now| {
        now.world_time = tick;
        edit_as_saved(now, |save| {
            let actors: Vec<String> = save["actors"]["control_type"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            for actor in actors {
                save["actors"]["acts"][actor] = act_row_with_idle_acts(3);
            }
        });
    });
    world.pack = Some(pack);
}

#[when(expr = "a seated round runs")]
fn a_seated_round_runs(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let rowan = Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    };
    let mut opened = open(&pack);
    let wall = opened.world().last_round_at.clone();
    opened
        .play(&rowan, &wall)
        .unwrap_or_else(|err| panic!("seated round: {}", err.as_str()));
}

#[then(expr = "the round ends at tick {int} and every actor's idle acts this window are {int}")]
fn every_actor_idle_acts(world: &mut World, tick: u64, acts: u32) {
    let now = world_of(world.pack.as_ref().unwrap());
    assert_eq!(now.world_time, tick);
    let actors = now.actors().query::<(ActorId, &component::Act)>();
    for (id, act) in actors.iter() {
        assert_eq!(act.idle_acts_this_window, acts, "{id}");
    }
}

#[then(expr = "docks_shop holds {int} lamp_oil")]
fn docks_shop_holds_lamp_oil(world: &mut World, count: u32) {
    let now = world_of(world.pack.as_ref().unwrap());
    let shop = "docks_shop".into();
    let stock = now.places().get::<component::Item>(&shop);
    let oil = stock.get("lamp_oil").copied();
    assert_eq!(oil, Some(count), "world_time {}", now.world_time);
}

#[given(expr = "attack targets defense + 10 and briar's defense is 12")]
fn attack_targets_defense(world: &mut World) {
    let pack = copy_pack("docks", "rules-attack");
    append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: attack
    when:
      verb: attack
      target: actor
    check:
      total: 1d20 + self.might
      success_at: "target.defense + 10"
    success:
      - flag:
          of: target
          id: hit
          value: true
    miss:
      - flag:
          of: target
          id: missed
          value: true
"#,
    );
    let briar = pack.join("cast/briar.md");
    let text = fs::read_to_string(&briar).unwrap();
    let next = text.replace(
        "  eligible: true\n",
        "  eligible: true\nmeters:\n  - id: defense\n    min: 0\n    base_cap: 20\n    max: 20\n    base: 12\n",
    );
    assert_ne!(text, next, "briar profile was not found");
    fs::write(&briar, next).unwrap();
    edit_save(&pack, |now| {
        put_meter(now, &entity::Id::Actor("briar".into()), "defense", 12);
        let mut actors = now.actors_mut();
        actors
            .get_mut::<component::Act>(&"briar".into())
            .set_busy(u64::MAX, None)
            .unwrap();
    });
    world.pack = Some(pack);
}

#[when(expr = "harbormouse attacks with a seeded total of 21")]
fn harbormouse_attacks(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let loaded = world_of(&pack);
    let face = roll::faces(
        &roll::Key {
            seed: loaded.seed.clone(),
            round_index: loaded.round_index + 1,
            rule_id: "verb:attack".to_string(),
            subject_id: Some("harbormouse".into()),
            index: 0,
        },
        0,
        1,
        20,
    )
    .unwrap_or_else(|err| panic!("faces {}", err.as_str()))[0];
    edit_save(&pack, |now| {
        edit_meter(now, "harbormouse", "might", |might| {
            (might.base, might.offset) = (21 - i64::from(face), 0);
        });
    });
    let mut opened = open(&pack);
    let nobody = Presence::default();
    opened
        .queue_player_verb(&nobody, &"harbormouse".into(), "attack", Some("briar"))
        .unwrap_or_else(|err| panic!("attack {}", err.code.as_str()));
    let wall = opened.world().last_round_at.clone();
    let outputs = opened
        .commit_then_fast_forward(&nobody, &wall)
        .unwrap_or_else(|err| panic!("attack {err}"));
    let reports = outputs.into_iter().next().unwrap().reports;
    let rolled = reports.into_iter().find_map(|report| match report {
        engine::roll::Report::Rolled { actor_id, roll } if actor_id == *"harbormouse" => Some(roll),
        _ => None,
    });
    let rolled = rolled.unwrap_or_else(|| panic!("Harbormouse did not roll"));
    assert_eq!(rolled.total, 21, "{rolled:?}");
    assert_eq!(rolled.success_at, 22, "{rolled:?}");
    world.scratch = match rolled.outcome {
        spookery::protocol::Outcome::Success => "success",
        spookery::protocol::Outcome::Partial => "partial",
        spookery::protocol::Outcome::Miss => "miss",
    }
    .to_string();
}

#[then(expr = "the outcome is a miss")]
fn the_outcome_is_a_miss(world: &mut World) {
    assert_eq!(world.scratch, "miss");
    let now = world_of(world.pack.as_ref().unwrap());
    let flags = flags_of(&now, "briar");
    assert!(flags.iter().any(|flag| flag == "missed"), "{flags:?}");
    assert!(!flags.iter().any(|flag| flag == "hit"), "{flags:?}");
}

#[given(expr = "a verb file where the word bogus appears in earlier text")]
fn bogus_appears_in_earlier_text(world: &mut World) {
    let pack = copy_pack("docks", "rules-bogus");
    append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: tale
    when:
      verb: tale
    do:
      - narrate:
          to: self
          text: the bogus tale
"#,
    );
    world.pack = Some(pack);
}

#[when(expr = "an effect key later in that file is bogus")]
fn an_effect_key_is_bogus(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    append(&pack.join("rules/verbs.yaml"), "      - bogus: true\n");
    stamp_pack_rev(&pack);
}

#[then(expr = "spook check reports that key at its own line and column")]
async fn check_reports_the_key(world: &mut World) {
    let text = fs::read_to_string(world.pack.as_ref().unwrap().join("rules/verbs.yaml")).unwrap();
    let (line, column) = locate_key(&text, "bogus:");
    let tale_line = text
        .lines()
        .position(|row| row.contains("the bogus tale"))
        .unwrap()
        + 1;
    assert_ne!(line as usize, tale_line);
    run(world, &["check", "--json"]).await;
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    let items: Vec<Value> = serde_json::from_str(world.stdout.trim())
        .unwrap_or_else(|err| panic!("{err}: {}\n{}", world.stdout, world.stderr));
    let hit = items
        .iter()
        .find(|item| {
            item["message"]
                .as_str()
                .is_some_and(|message| message.contains("unknown 'bogus'"))
        })
        .unwrap_or_else(|| panic!("{}", world.stdout));
    assert_eq!(hit["file"], "rules/verbs.yaml");
    assert_eq!(hit["line"], line);
    assert_eq!(hit["column"], column, "{hit}");
    assert_ne!(hit["line"].as_u64(), Some(tale_line as u64));
}

const SECOND_KEY: &str = "flag: { of: self, key: second_key";

#[given(expr = "a verb whose {word} {word} holds two keys in one line")]
fn two_keys_in_one_line(world: &mut World, field: String, form: String) {
    let pack = copy_pack("docks", "rules-two");
    let (first, second) = match field.as_str() {
        "if" => ("has_item: { subject: self, id: knife }", "is: true }"),
        "do" => ("narrate: { to: self, text: hi }", "value: true }"),
        other => panic!("no {other} field"),
    };
    let (dash, indent) = if form == "list" {
        ("- ", "  ")
    } else {
        ("", "")
    };
    let rule = format!(
        "\n  - id: both\n    when:\n      verb: both\n    {field}:\n      {dash}{first}\n      {indent}{SECOND_KEY}, {second}\n"
    );
    append(&pack.join("rules/verbs.yaml"), &rule);
    stamp_pack_rev(&pack);
    world.pack = Some(pack);
}

#[then(expr = "spook check reports two keys in one condition or effect at the second key")]
async fn check_reports_two_keys(world: &mut World) {
    let text = fs::read_to_string(world.pack.as_ref().unwrap().join("rules/verbs.yaml")).unwrap();
    let (line, _) = locate_key(&text, SECOND_KEY);
    run(world, &["check"]).await;
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    let want = format!(
        "rules/verbs.yaml:{line}: two keys in one condition or effect; give each condition or effect its own dash; combine conditions with all:, any: or not: (SCHEMA)"
    );
    assert!(world.stderr.contains(&want), "{}", world.stderr);
}

#[given(expr = "steady costs 5 vigor and the sitter has 1")]
fn steady_costs_five_vigor(world: &mut World) {
    let pack = copy_pack("docks", "rules-cost");
    append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: steady
    when:
      verb: steady
    cost:
      meters:
        vigor: 5
    do: []
"#,
    );
    edit_save(&pack, |now| {
        edit_meter(now, "rowan", "vigor", |vigor| {
            (vigor.base, vigor.offset) = (1, 0)
        });
    });
    world.pack = Some(pack);
}

#[when(expr = "the seated PC plays steady")]
fn the_seated_pc_plays_steady(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let mut opened = open(&pack);
    let played = opened.queue_player_verb(&Presence::default(), &"rowan".into(), "steady", None);
    world.scratch = match played {
        Err(err) => format!("{} {}", err.code.as_str(), err.detail.unwrap_or_default()),
        Ok(_) => "queued".to_string(),
    };
}

#[then(expr = "the refusal is CANT_AFFORD naming vigor and world_time stays 1080")]
fn the_refusal_is_cant_afford(world: &mut World) {
    assert_eq!(
        world.scratch,
        format!("{} vigor", Reason::CantAfford.as_str())
    );
    let now = world_of(world.pack.as_ref().unwrap());
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.round_index, 0);
}

#[given(
    expr = "a habit cast:mira\\/leave_door_note and a place rule place:docks_street\\/travel while the PC stays in the shop"
)]
fn a_far_place_rule_while_the_pc_stays(world: &mut World) {
    let pack = copy_pack("docks", "rules-full-id");
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
          of: self
          id: took_the_street
          value: true
",
    );
    edit_save(&pack, |_| {});
    let now = world_of(&pack);
    assert_eq!(place_of(&now, "rowan"), "docks_shop");
    world.pack = Some(pack);
}

#[when(expr = "a PC is gated on those full ids")]
fn a_pc_is_gated_on_those_full_ids(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let mut opened = open(&pack);
    let mut codes = Vec::new();
    for verb_id in ["cast:mira/leave_door_note", "place:docks_street/travel"] {
        match opened.queue_player_verb(&Presence::default(), &"rowan".into(), verb_id, None) {
            Err(miss) if miss.code == Reason::NotInSpec => {
                codes.push(miss.code.as_str());
            }
            other => panic!("{verb_id}: {other:?}"),
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
            Reason::NotInSpec.as_str(),
            Reason::NotInSpec.as_str()
        )
    );
    let now = world_of(world.pack.as_ref().unwrap());
    assert_eq!(place_of(&now, "rowan"), "docks_shop");
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.round_index, 0);
    let notes = now.globals().get::<Note>(&GlobalId);
    assert!(
        !notes.contains("note_mira_door"),
        "the habit note was stored"
    );
    assert!(
        !now.actors()
            .get::<component::Flag>(&"rowan".into())
            .contains("took_the_street"),
        "the far place rule ran"
    );
}

#[given(expr = "use on tonic targets an item and the shared use takes no target")]
fn use_on_tonic_targets_an_item(world: &mut World) {
    let pack = copy_pack("docks", "rules-owned");
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
        put_items(now, &entity::Id::Actor("rowan".into()), "tonic", 1);
    });
    world.pack = Some(pack);
}

#[when(expr = "the seated PC queues use tonic")]
fn the_seated_pc_queues_use_tonic(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    let mut opened = open(&pack);
    let played =
        opened.queue_player_verb(&Presence::default(), &"rowan".into(), "use", Some("tonic"));
    world.scratch = match played {
        Ok((queued, _)) => {
            let intent = opened
                .queued_intents()
                .iter()
                .find(|intent| intent.id == queued.id)
                .unwrap();
            format!("{:?} {}", intent.target, intent.verb_id)
        }
        Err(err) => err.code.as_str().to_string(),
    };
}

#[then(expr = "the queued target is that item and the save is unchanged")]
fn the_queued_target_is_that_item(world: &mut World) {
    assert_eq!(world.scratch, "Item(\"tonic\") use");
    let now = world_of(world.pack.as_ref().unwrap());
    assert_eq!(place_of(&now, "rowan"), "docks_shop");
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.round_index, 0);
}

#[given(expr = "a verb with a time cost and a nested check")]
fn a_verb_with_time_and_a_nested_check(world: &mut World) {
    let pack = copy_pack("docks", "rules-unused");
    append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: linger
    when:
      verb: linger
    cost:
      ticks: 10
    do:
      - check:
          total: 2d6 + self.vigor
          success_at: 2
          success:
            - meter:
                of: self
                id: vigor
                add: 1
"#,
    );
    edit_save(&pack, |_| {});
    world.pack = Some(pack);
}

#[then(expr = "spook check is quiet about the time and the nested check")]
async fn check_is_quiet_about_time_and_the_nested_check(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    open(&pack);
    run(world, &["check"]).await;
    assert_eq!(world.code, Some(0), "{}\n{}", world.stdout, world.stderr);
    assert!(!world.stderr.contains("cost time"), "{}", world.stderr);
    assert!(!world.stderr.contains("nested check"), "{}", world.stderr);
}

#[given(
    expr = "briar's cast file holds a verb \"ask\" that targets an actor and sets the world flag passage, and Mira's door note is not due"
)]
fn briar_holds_ask_with_miras_note_not_due(world: &mut World) {
    let pack = copy_pack("docks", "rules-scoped-here");
    edit_save(&pack, |now| {
        edit_meter(now, "mira", "shop_day", |shop_day| shop_day.base = 0);
    });
    world.pack = Some(pack);
}

#[given(
    expr = "briar's cast file holds a verb \"ask\" that targets an actor, and the save puts briar in the lane"
)]
fn briar_holds_ask_in_the_lane(world: &mut World) {
    let pack = copy_pack("docks", "rules-scoped-gone");
    edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"briar".into())
            .set("mill_lane")
            .unwrap();
    });
    world.pack = Some(pack);
}

#[when(regex = r"^harbormouse, in the yard with briar, plays (.+)$")]
fn harbormouse_in_the_yard_plays(world: &mut World, line: String) {
    let now = world_of(world.pack.as_ref().unwrap());
    for actor in ["harbormouse", "briar"] {
        assert_eq!(place_of(&now, actor), "gate_yard");
    }
    play_harbormouse_line(world, &line);
}

#[when(regex = r"^harbormouse, alone in the yard, plays (.+)$")]
fn harbormouse_alone_plays(world: &mut World, line: String) {
    let now = world_of(world.pack.as_ref().unwrap());
    let actors = now.actors().query::<(ActorId, &component::Place)>();
    let yard = actors
        .iter()
        .filter(|(_, place)| *place == &"gate_yard".into());
    assert_eq!(
        yard.map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
        ["harbormouse"]
    );
    play_harbormouse_line(world, &line);
}

#[then(expr = "the verb is refused NOT_IN_SPEC before the round and the save is unchanged")]
fn ask_is_refused_before_the_round(world: &mut World) {
    assert_eq!(world.scratch, Reason::NotInSpec.as_str());
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(
        host::read_files(pack),
        world.before,
        "the ask wrote the pack"
    );
}

/// Harbormouse plays `line` seated, keeping the pack's bytes from before. A queued line plays its
/// round and keeps the lines it told, else the scratch holds the refusal code.
fn play_harbormouse_line(world: &mut World, line: &str) {
    let pack = world.pack.clone().unwrap();
    world.before = host::read_files(&pack);
    let (verb, target) = split_line(line);
    let (seated, mut opened) = (seated_harbormouse(), open(&pack));
    if let Err(refusal) = opened.queue_player_verb(&seated, &"harbormouse".into(), verb, target) {
        world.scratch = refusal.code.as_str().to_string();
        return;
    }
    let wall = opened.world().last_round_at.clone();
    let output = opened
        .play(&seated, &wall)
        .unwrap_or_else(|err| panic!("{line}: {err}"));
    let lines = output.lines.into_iter();
    world.told = lines.map(|line| (line.to.into(), line.text)).collect();
}

fn seated_harbormouse() -> Presence {
    Presence {
        players: 1,
        actors_present: vec!["harbormouse".into()],
    }
}

async fn run(world: &mut World, args: &[&str]) {
    let pack = crate::support::pack_arg(world.pack.as_ref().unwrap());
    let mut full = Vec::with_capacity(args.len() + 1);
    full.extend_from_slice(args);
    full.push(pack.as_str());
    host::run(world, &full).await;
}

/// The world owns the pack lock, so the caller writes while holding it.
pub(super) fn open(pack: &Path) -> spookery_engine::Session {
    let lock = spookery_engine::lock_pack(pack).unwrap();
    spookery_engine::Session::resume(pack, lock).unwrap_or_else(|err| {
        panic!(
            "open {}: {}",
            err.as_str(),
            spookery_engine::check_pack(pack)
                .iter()
                .map(|diagnostic| diagnostic.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn locate_key(text: &str, key: &str) -> (u64, u64) {
    for (index, line) in text.lines().enumerate() {
        if let Some(column) = line.find(key) {
            return ((index + 1) as u64, (column + 1) as u64);
        }
    }
    panic!("missing {key}");
}
