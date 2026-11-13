use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::{faces, RollKey};
use spookery_engine::{component, MoveReport};

use crate::{id_named, World};
use spookery_engine::testing::{self as support, read_round};

#[given(expr = "a round where Lark rolls grit")]
fn a_round_where_lark_rolls_grit(world: &mut World) {
    let clean = lark_pack();
    let played = support::fresh_dir("lark-alone");
    support::copy_dir(&clean, &played);
    let (faces, total) = lark_roll(&played, false);
    let _ = fs::remove_dir_all(played);
    world.scratch = format!("{faces}|{total}");
    world.pack = Some(clean);
}

#[when(expr = "the same round is played again with an extra unrelated NPC roll queued first")]
fn the_same_round_is_played_again_with_an_extra_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let (faces, total) = lark_roll(&pack, true);
    world.intent_id = format!("{faces}|{total}");
}

#[then(expr = "Lark's faces and total are identical")]
fn larks_faces_and_total_are_identical(world: &mut World) {
    assert_eq!(world.intent_id, world.scratch);
}

#[given(expr = "ten rolling intents in one round")]
fn ten_rolling_intents_in_one_round(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "ten-dice");
    let intents = (0..10)
        .map(|index| intent(&format!("grit-{index}"), "rowan", "grit", None))
        .collect();
    let (_state, rolls, round) = play_queued(&pack, &[], intents);
    world.scratch = round.to_string();
    world.intent_id = rolls.len().to_string();
    world.pack = Some(pack);
}

#[then(expr = "all ten resolve and none is refused NotInSpec")]
fn all_ten_resolve_and_none_is_refused_not_in_spec(world: &mut World) {
    assert_eq!(world.intent_id, "10");
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    let rejected = round["refused"].as_array().unwrap();
    assert!(
        rejected.iter().all(|item| item["code"] != "NOT_IN_SPEC"),
        "{rejected:?}"
    );
}

#[given(expr = "a played d20 fight")]
fn a_played_d20_fight(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "d20-fight");
    let intents = vec![intent("strike-lark", "lark", "strike", Some("briar"))];
    let (state, _rolls, _round) = play_queued(&pack, &[], intents);
    world.oil = spookery_engine::emit_save(&state).unwrap().into_bytes();
    world.pack = Some(pack);
}

#[then(expr = "spook verify passes and re-derives every roll")]
fn spook_verify_passes_and_re_derives_every_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    spookery_engine::verify_pack(pack, None).expect("verify");
    let report =
        spookery_engine::testing::replay_pack(pack).unwrap_or_else(|err| panic!("{err:?}"));
    let line = spookery_engine::testing::journal::read_record_line(pack, "round-0001")
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let round: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        spookery_engine::testing::journal::blake2s_tagged(&report.now_yaml),
        round["state_hash"].as_str().unwrap()
    );
}

#[given(expr = "a docks row overreach that adds {int} lamp oil and then removes {int}")]
fn a_docks_row_overreach(world: &mut World, add: i64, remove: i64) {
    let pack = support::copy_pack("oneshot-docks", "rollback");
    support::append(
        &pack.join("rules/verbs.yaml"),
        &format!(
            "
  - id: overreach
    when:
      verb: overreach
    do:
      - item:
          of: place:docks_shop
          id: lamp_oil
          add: {add}
      - item:
          of: place:docks_shop
          id: lamp_oil
          add: -{remove}
"
        ),
    );
    support::stamp_pack_rev(&pack);
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

#[given(expr = "a docks row spilled that rolls grit and then removes {int} lamp oil")]
fn a_docks_row_spilled(world: &mut World, remove: i64) {
    let pack = support::copy_pack("oneshot-docks", "spilled");
    support::append(
        &pack.join("rules/verbs.yaml"),
        &format!(
            "
  - id: spilled
    when:
      verb: spilled
    roll:
      check: grit
      threshold: 0
    do:
      - item:
          of: place:docks_shop
          id: lamp_oil
          add: -{remove}
"
        ),
    );
    support::stamp_pack_rev(&pack);
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Mira plays {word}")]
fn mira_plays(world: &mut World, move_name: String) {
    let pack = world.pack.as_ref().unwrap().clone();
    let id = format!("{move_name}-1");
    let (state, rolls, round) =
        play_queued(&pack, &[], vec![intent(&id, "mira", &move_name, None)]);
    let oil = state
        .places()
        .items
        .get(&("docks_shop".into(), "lamp_oil".to_string()))
        .copied()
        .map(i64::from)
        .unwrap_or(-1);
    world.oil = oil.to_string().into_bytes();
    world.intent_id = id;
    world.scratch = round.to_string();
    world.count = rolls
        .iter()
        .filter(|report| match report {
            MoveReport::Rolled { actor_id, .. } => actor_id == "mira",
            MoveReport::Refused { .. } => false,
        })
        .count();
}

#[then(expr = "lamp oil stays {int} and the round names {word}")]
fn lamp_oil_stays_and_the_round_names(world: &mut World, oil: i64, reason: String) {
    assert_refusal(world, oil, &reason);
}

#[then(expr = "lamp oil stays {int}, the round names {word}, and no roll is reported")]
fn lamp_oil_stays_the_round_names_and_no_roll_is_reported(
    world: &mut World,
    oil: i64,
    reason: String,
) {
    assert_refusal(world, oil, &reason);
    assert_eq!(world.count, 0, "a refused move reported a roll");
}

#[given(expr = "the docks pack, where Mira's door-note habit is due")]
fn the_docks_pack_where_miras_habit_is_due(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "habit-ineligible");
    let opened = spookery_engine::World::peek(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    let view = opened
        .perceive(&spookery_engine::Presence::default(), "mira")
        .unwrap_or_else(|err| panic!("perceive: {err}"));
    assert!(view.habits.iter().any(|habit| habit == "leave_door_note"));
    world.pack = Some(pack);
}

#[when(expr = "the GM queues mira wait and an empty round plays")]
fn the_gm_queues_mira_wait(world: &mut World) {
    let wait = intent("gm-wait", "mira", "wait", None);
    let (_state, _rolls, round) = play_queued(world.pack(), &[], vec![wait]);
    world.scratch = round.to_string();
}

#[then(
    expr = "the wait lands, and Mira's habit is refused INELIGIBLE because she already acted this round"
)]
fn the_wait_lands_and_miras_habit_is_ineligible(world: &mut World) {
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    let rules: Vec<&str> = round["extra_steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|step| step["rule"].as_str())
        .collect();
    assert_eq!(rules, ["verb:wait"], "{round}");
    let refused = round["refused"].as_array().unwrap();
    assert_eq!(refused.len(), 1, "{round}");
    assert_eq!(refused[0]["code"], "INELIGIBLE", "{round}");
    assert_ne!(refused[0]["id"], "gm-wait", "{round}");
    let state = support::state_of(world.pack());
    assert_eq!(state.actors().acts["mira"].last_act_round, 1);
    assert!(state.notes().is_empty(), "{:?}", state.notes());
}

#[when(expr = "Lark and Briar strike each other")]
fn lark_and_briar_strike_each_other(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "mutual-strike");
    let (_state, rolls, _round) = play_queued(
        &pack,
        &[],
        vec![
            intent("lark-strike", "lark", "strike", Some("briar")),
            intent("briar-strike", "briar", "strike", Some("lark")),
        ],
    );
    world.pack = Some(pack);
    world.scratch = rolls
        .iter()
        .filter_map(|report| match report {
            MoveReport::Rolled { actor_id, roll } => Some(format!(
                "{actor_id}={}:{}:{}",
                roll.faces
                    .iter()
                    .map(|face| face.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                roll.total,
                roll.target
            )),
            MoveReport::Refused { .. } => None,
        })
        .collect::<Vec<_>>()
        .join(";");
}

#[then(expr = "each strike is scored on the other's own roll")]
fn each_strike_is_scored_on_the_others_own_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let lark = strike_line(&world.scratch, "lark");
    let briar = strike_line(&world.scratch, "briar");
    let lark_once = indexed_roll(pack, "lark", 0);
    let lark_again = indexed_roll(pack, "lark", 1);
    let briar_once = indexed_roll(pack, "briar", 0);
    let briar_again = indexed_roll(pack, "briar", 1);
    assert_ne!(lark_once.1, lark_again.1, "lark's two indexes collide");
    assert_ne!(briar_once.1, briar_again.1, "briar's two indexes collide");
    assert_eq!(lark.0, lark_once.0, "lark was rolled twice");
    assert_eq!(briar.0, briar_once.0, "briar was rolled twice");
    assert_eq!(lark.1, lark_once.1);
    assert_eq!(briar.1, briar_once.1);
    assert_eq!(lark.2, briar.1, "lark's target is not briar's roll");
    assert_eq!(briar.2, lark.1, "briar's target is not lark's roll");
}

#[given(expr = "max_effects_per_intent is 1 and a row with 2 effects")]
fn max_effects_per_intent_is_1_and_a_row_with_2_effects(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "effect-cap");
    set_budget(&pack, "max_effects_per_intent", 1);
    support::append(
        &pack.join("rules/verbs.yaml"),
        "
  - id: two_stock
    when:
      verb: two_stock
    do:
      - item:
          of: place:docks_shop
          id: lamp_oil
          add: 1
      - item:
          of: place:docks_shop
          id: lamp_oil
          add: 1
",
    );
    support::stamp_pack_rev(&pack);
    make_mira_ineligible(&pack);
    let (state, _rolls, round) =
        play_queued(&pack, &[], vec![intent("two-1", "mira", "two_stock", None)]);
    world.oil = format!(
        "{}",
        state
            .places()
            .items
            .get(&("docks_shop".into(), "lamp_oil".to_string()))
            .copied()
            .map(i64::from)
            .unwrap_or(-1)
    )
    .into_bytes();
    world.scratch = round.to_string();
    world.pack = Some(pack);
}

#[then(expr = "no effect from that row applies and the round log names the refusal")]
fn no_effect_from_that_row_applies(world: &mut World) {
    assert_eq!(String::from_utf8_lossy(&world.oil), "4");
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    assert!(round["effects"].as_array().unwrap().is_empty(), "{round}");
    let rejected = round["refused"].as_array().unwrap();
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == "two-1" && item["code"] == "BUDGET"),
        "{round}"
    );
}

#[given(
    expr = "a docks pack where Aldric's cast file holds a ten-tick verb \"ask\" that lowers the target's rest"
)]
fn a_docks_pack_where_aldric_holds_a_timed_verb(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "timed-target");
    let path = pack.join("cast/aldric.md");
    let text = fs::read_to_string(&path).unwrap();
    let rule = "  eligible: false
rules:
  - id: ask
    when: { verb: ask, target: actor }
    cost: { ticks: 10 }
    do: [meter: { of: target, key: rest, add: -1 }]
---";
    let next = text.replacen("  eligible: false\n---", rule, 1);
    assert_ne!(next, text, "aldric's front matter moved");
    fs::write(&path, next).unwrap();
    support::stamp_pack_rev(&pack);
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Rowan asks Aldric and the table plays until the action finishes")]
fn rowan_asks_aldric_until_it_finishes(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let lock = spookery_engine::lock_pack(&pack).unwrap();
    let mut table = spookery_engine::World::resume(&pack, lock).unwrap_or_else(|e| panic!("{e}"));
    let rowan = spookery_engine::Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    };
    table
        .queue_player_move(&rowan, &"rowan".into(), "ask", Some("aldric"))
        .unwrap_or_else(|refusal| panic!("ask aldric: {refusal:?}"));
    let wall = table.state().last_round_at.clone();
    for round in 1..=6 {
        let output = table.play(&rowan, &wall);
        output.unwrap_or_else(|err| panic!("round {round}: {err}"));
        let state = table.state();
        if !state.actors().acts["rowan"].is_busy(state.world_time) {
            break;
        }
        assert!(round < 6, "Rowan is still busy after {round} rounds");
    }
}

#[then(expr = "{word}'s rest is {int}")]
fn rest_is(world: &mut World, actor: String, rest: i64) {
    let actor = spookery_engine::entity::Id::Actor(id_named(world.pack(), &actor).into());
    let now = support::state_of(world.pack());
    let held = now.entities().get::<component::Meter>(&actor).value("rest");
    assert_eq!(held, Ok(rest), "{}", world.scratch);
}

#[given(
    expr = "a docks pack whose verbs hold a ten-tick verb \"haul\" that takes {int} lamp oil from the actor"
)]
fn a_docks_pack_with_a_timed_haul(world: &mut World, oil: i64) {
    let pack = support::copy_pack("oneshot-docks", "timed-refused");
    support::append(
        &pack.join("rules/verbs.yaml"),
        &format!(
            "
  - id: haul
    when: {{ verb: haul }}
    cost: {{ ticks: 10 }}
    do: [item: {{ of: self, id: lamp_oil, add: -{oil} }}]
"
        ),
    );
    support::stamp_pack_rev(&pack);
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Rowan hauls with no lamp oil and the table plays until the action finishes")]
fn rowan_hauls_until_it_finishes(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut table = support::resume(&pack);
    let rowan = spookery_engine::Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    };
    table
        .queue_player_move(&rowan, &"rowan".into(), "haul", None)
        .unwrap_or_else(|refusal| panic!("haul: {refusal:?}"));
    let wall = table.state().last_round_at.clone();
    world.scratch.clear();
    for round in 1..=6 {
        if let Err(err) = table.play(&rowan, &wall) {
            world.scratch = format!("round {round} failed: {err}");
            return;
        }
        let state = table.state();
        if !state.actors().acts["rowan"].is_busy(state.world_time) {
            break;
        }
    }
    world.count = usize::try_from(table.state().round_index).unwrap();
}

#[then(
    expr = "the finishing round commits with finish:rowan refused MISSING_ITEM, Rowan is no longer busy, and spook verify passes"
)]
fn the_refused_finish_commits(world: &mut World) {
    assert!(world.scratch.is_empty(), "{}", world.scratch);
    let pack = world.pack();
    let round = read_round(pack, &format!("round-{:04}", world.count));
    let refused = round["refused"].as_array().unwrap();
    assert!(
        refused
            .iter()
            .any(|item| item["id"] == "finish:rowan" && item["code"] == "MISSING_ITEM"),
        "{round}"
    );
    let saved = spookery_engine::World::peek(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let state = saved.state();
    assert!(
        !state.actors().acts["rowan"].is_busy(state.world_time),
        "{round}"
    );
    spookery_engine::verify_pack(pack, None).unwrap_or_else(|err| panic!("verify: {err}"));
}

#[given(
    expr = "a docks pack whose \"pick\" rule rolls grit against a threshold that divides by zero"
)]
fn a_docks_pack_with_a_broken_pick(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "broken-threshold");
    // Grit starts at 1, so the divisor is 0 only at play, past `spook check`.
    support::append(
        &pack.join("rules/verbs.yaml"),
        "
  - id: pick
    when: { verb: pick }
    roll: { check: grit, threshold: \"10 / (self.grit - 1)\" }
    do: []
",
    );
    support::stamp_pack_rev(&pack);
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Rowan walks to the street and Wren plays pick in the same round")]
fn rowan_walks_and_wren_picks(world: &mut World) {
    let pick = intent("pick-1", "wren", "pick", None);
    let (_state, round) = rowan_walks(world.pack(), vec![pick]);
    world.scratch = round.to_string();
}

#[then(expr = "Wren's pick is refused {word} and Rowan is on the street")]
fn wrens_pick_is_refused(world: &mut World, code: String) {
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    let refused = round["refused"].as_array().unwrap();
    assert!(
        refused
            .iter()
            .any(|item| item["id"] == "pick-1" && item["code"] == code.as_str()),
        "{round}"
    );
    let saved = spookery_engine::World::peek(world.pack()).unwrap();
    assert_eq!(saved.state().actors().place["rowan"], "docks_street");
}

#[then(expr = "the next round plays and spook verify passes")]
fn the_next_round_plays_and_verify_passes(world: &mut World) {
    let pack = world.pack();
    let lock = spookery_engine::lock_pack(pack).unwrap();
    let mut table =
        spookery_engine::World::resume(pack, lock).unwrap_or_else(|err| panic!("open: {err}"));
    let wall = table.state().last_round_at.clone();
    table
        .queue_gm_intent(
            &spookery_engine::Presence::default(),
            intent("wait-1", "wren", "wait", None),
            || panic!("the GM chose the id"),
        )
        .unwrap_or_else(|code| panic!("queue: {code}"));
    table
        .play(&spookery_engine::Presence::default(), &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
    assert_eq!(table.state().round_index, 2);
    drop(table);
    spookery_engine::verify_pack(pack, None).unwrap_or_else(|err| panic!("verify: {err}"));
}

#[given(
    expr = "a docks pack where the shop's \"rummage\" costs {int} rest and the shared \"rummage\" costs {int}"
)]
fn a_docks_pack_with_two_rummage_rules(world: &mut World, shop: i64, shared: i64) {
    let pack = support::copy_pack("oneshot-docks", "rummage");
    let rule = format!(
        "  - id: rummage
    when: {{ verb: rummage }}
    cost: {{ meters: {{ rest: {shop} }} }}
    roll: {{ check: grit, threshold: 0 }}
    do: []"
    );
    add_place_rules(&pack, "docks_shop", &rule);
    let rule = format!(
        "
  - id: rummage
    when: {{ verb: rummage }}
    cost: {{ meters: {{ rest: {shared} }} }}
    do: []
"
    );
    support::append(&pack.join("rules/verbs.yaml"), &rule);
    support::stamp_pack_rev(&pack);
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Rowan walks to the street and rummages in the same round")]
fn rowan_walks_and_rummages(world: &mut World) {
    let rummage = intent("rummage", "rowan", "rummage", None);
    let (_state, round) = rowan_walks(world.pack(), vec![rummage]);
    world.scratch = round.to_string();
}

#[then(expr = "the journal records {string} and not {string}")]
fn the_journal_records_and_not(world: &mut World, ran: String, skipped: String) {
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    let rules: Vec<&str> = round["extra_steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|step| step["rule"].as_str())
        .collect();
    assert!(rules.contains(&ran.as_str()), "{round}");
    assert!(!rules.contains(&skipped.as_str()), "{round}");
}

#[given(
    expr = "max_effects_per_intent is 1 and a street rule that sets 2 flags when Rowan arrives"
)]
fn a_street_rule_over_the_cap(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "trigger-cap");
    set_budget(&pack, "max_effects_per_intent", 1);
    let rule = "  - id: greeted
    when: { change: { arrive: here, subject: rowan } }
    do:
      - flag: { of: self, key: greeted, value: true }
      - flag: { of: self, key: welcomed, value: true }";
    add_place_rules(&pack, "docks_street", rule);
    support::stamp_pack_rev(&pack);
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Rowan walks to the street")]
fn rowan_walks_to_the_street(world: &mut World) {
    let (state, round) = rowan_walks(world.pack(), Vec::new());
    let rowan = "rowan".into();
    let flags = state.actors().get::<component::Flag>(&rowan);
    world.count = flags.iter().count();
    world.scratch = round.to_string();
}

#[then(expr = "neither flag is set and the round log names the change rule's {word} refusal")]
fn neither_flag_is_set(world: &mut World, code: String) {
    assert_eq!(world.count, 0, "flags were set");
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    let rejected = round["refused"].as_array().unwrap();
    let id = "trigger:place:docks_street/greeted:rowan";
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == id && item["code"] == code),
        "{round}"
    );
}

#[given(expr = "the docks pack with Rowan seated in the shop")]
fn the_docks_pack_with_rowan_seated(world: &mut World) {
    world.pack = Some(support::copy_pack("oneshot-docks", "exit-touch"));
}

#[when(expr = "Bram and Aldric wait and then Mira travels through the {string} exit")]
fn two_wait_and_mira_travels(world: &mut World, exit: String) {
    let intents = vec![
        intent("wait-bram", "bram", "wait", None),
        intent("wait-aldric", "aldric", "wait", None),
        intent("walk-mira", "mira", "travel", Some(&exit)),
    ];
    let (_state, _rolls, round) = play_queued(world.pack(), &["rowan"], intents);
    world.scratch = round.to_string();
}

#[then(expr = "Mira's travel is picked ahead of both waits")]
fn miras_travel_is_picked_first(world: &mut World) {
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    let rules: Vec<&str> = round["extra_steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|step| step["rule"].as_str())
        .filter(|rule| ["verb:travel", "verb:wait"].contains(rule))
        .collect();
    // Mira's walk starts first and finishes after the waits, when its ticks pass.
    assert_eq!(
        rules,
        vec!["verb:travel", "verb:wait", "verb:wait", "verb:travel"],
        "{round}"
    );
}

#[then(expr = "Mira has visited the Docks Shop")]
fn mira_has_visited_the_shop(world: &mut World) {
    let saved = spookery_engine::World::peek(world.pack()).unwrap();
    let actors = &saved.state().actors();
    assert_eq!(actors.place["mira"], "docks_shop");
    assert!(
        actors
            .visited
            .contains_key(&("mira".into(), "docks_shop".to_string())),
        "{:?}",
        actors.visited
    );
}

#[given(expr = "the d20 pack allows {int} intent per round")]
fn the_d20_pack_allows_intents(world: &mut World, limit: u64) {
    let pack = support::copy_pack("sample-d20", "intent-limit");
    set_budget(&pack, "max_intents_applied", limit);
    support::stamp_pack_rev(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Lark waits and Briar waits in the same round")]
fn lark_and_briar_wait(world: &mut World) {
    let intents = vec![
        intent("lark-wait", "lark", "wait", None),
        intent("briar-wait", "briar", "wait", None),
    ];
    let (_state, _rolls, round) = play_queued(world.pack(), &[], intents);
    world.scratch = round.to_string();
}

#[then(expr = "Briar's wait is refused BUDGET and nothing is left queued")]
fn briars_wait_is_refused_budget(world: &mut World) {
    assert_budget_refusals(world, &["briar-wait"]);
}

#[given(
    expr = "the docks pack allows {int} state key per round and a row \"overset\" that sets 2 flags"
)]
fn the_docks_pack_allows_keys(world: &mut World, limit: u64) {
    let pack = support::copy_pack("oneshot-docks", "key-limit");
    set_budget(&pack, "max_state_keys_mutated", limit);
    support::append(
        &pack.join("rules/verbs.yaml"),
        "
  - id: overset
    when: { verb: overset }
    do:
      - flag: { of: world, key: greeted, value: true }
      - flag: { of: world, key: welcomed, value: true }
",
    );
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Rowan plays overset and then Wren waits in the same round")]
fn rowan_oversets_and_wren_waits(world: &mut World) {
    let intents = vec![
        intent("rowan-overset", "rowan", "overset", None),
        intent("wren-wait", "wren", "wait", None),
    ];
    let (_state, _rolls, round) = play_queued(world.pack(), &[], intents);
    world.scratch = round.to_string();
}

#[then(expr = "Rowan's overset and Wren's wait are both refused BUDGET and nothing is left queued")]
fn rowans_overset_and_wrens_wait_are_refused_budget(world: &mut World) {
    assert_budget_refusals(world, &["rowan-overset", "wren-wait"]);
}

/// Each of `ids` is refused `BUDGET` in the round, and the save keeps no queue.
fn assert_budget_refusals(world: &World, ids: &[&str]) {
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    let refused = round["refused"].as_array().unwrap();
    for id in ids {
        assert!(
            refused
                .iter()
                .any(|item| item["id"] == *id && item["code"] == "BUDGET"),
            "{id}: {round}"
        );
    }
    let saved = spookery_engine::World::peek(world.pack()).unwrap();
    assert!(saved.queued_intents().is_empty());
}

/// Set the budget's `key` line to `value`.
fn set_budget(pack: &Path, key: &str, value: u64) {
    let path = pack.join("rules/budget.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let old = text
        .lines()
        .find(|line| line.starts_with(&format!("{key}:")))
        .unwrap_or_else(|| panic!("no {key} line"));
    fs::write(&path, text.replacen(old, &format!("{key}: {value}"), 1)).unwrap();
}

/// Rowan, seated, takes the street exit, then plays `then` in the same round.
fn rowan_walks(
    pack: &Path,
    then: Vec<spookery_protocol::QueueArgs>,
) -> (spookery_engine::State, Value) {
    let mut intents = vec![intent("walk", "rowan", "travel", Some("docks_street"))];
    intents.extend(then);
    let (state, _rolls, round) = play_queued(pack, &["rowan"], intents);
    assert_eq!(state.actors().place["rowan"], "docks_street", "{round}");
    (state, round)
}

/// Give `place` its own `rules:` list, written just before its exits.
fn add_place_rules(pack: &Path, place: &str, rules: &str) {
    let path = pack.join(format!("world/places/{place}.md"));
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replacen("exits:", &format!("rules:\n{rules}\nexits:"), 1);
    assert_ne!(next, text, "{place} has no exits");
    fs::write(&path, next).unwrap();
}

fn assert_refusal(world: &World, oil: i64, reason: &str) {
    assert_eq!(String::from_utf8_lossy(&world.oil), oil.to_string());
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    assert!(round["effects"].as_array().unwrap().is_empty(), "{round}");
    let rejected = round["refused"].as_array().unwrap();
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == world.intent_id && item["code"] == reason),
        "{round}"
    );
}

fn strike_line(scratch: &str, actor: &str) -> (String, i64, i64) {
    let prefix = format!("{actor}=");
    let line = scratch
        .split(';')
        .find(|item| item.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no roll for {actor} in {scratch}"));
    let rest = line.trim_start_matches(&prefix);
    let mut parts = rest.split(':');
    let faces = parts.next().unwrap().to_string();
    let total = parts.next().unwrap().parse().unwrap();
    let target = parts.next().unwrap().parse().unwrap();
    (faces, total, target)
}

fn indexed_roll(pack: &Path, actor_id: &str, roll_index: u32) -> (String, i64) {
    let world = spookery_engine::World::peek(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let check = world.canon().checks.get("strike").unwrap();
    let def = world.canon().meters.get(&check.stat).unwrap();
    let actor = actor_id.into();
    let live = world
        .state()
        .actors()
        .get::<component::Meter>(&actor)
        .get(&check.stat)
        .unwrap_or_else(|| panic!("missing {actor_id}'s {}", check.stat));
    let stat = (live.base + live.offset).clamp(def.min, def.max);
    let rolled = faces(
        &RollKey {
            world_id: world.state().world_id.clone(),
            round_index: world.state().round_index,
            rule_id: "verb:strike".to_string(),
            subject_id: Some(actor_id.into()),
            index: roll_index,
        },
        0,
        check.dice_count,
        check.dice_sides,
    )
    .unwrap_or_else(|err| panic!("roll: {err}"));
    let total = rolled.iter().map(|face| i64::from(*face)).sum::<i64>() + stat;
    let faces = rolled
        .iter()
        .map(|face| face.to_string())
        .collect::<Vec<_>>()
        .join(",");
    (faces, total)
}

fn lark_pack() -> PathBuf {
    let pack = support::copy_pack("sample-d20", "lark-grit");
    support::append(
        &pack.join("rules/meters.yaml"),
        "
  - id: grit
    min: 0
    base_cap: 100
    max: 110
    base: 1
",
    );
    support::append(
        &pack.join("rules/checks.yaml"),
        "
  - id: grit
    dice:
      count: 2
      sides: 6
    meter: grit
",
    );
    support::append(
        &pack.join("rules/verbs.yaml"),
        "
  - id: grit
    when:
      verb: grit
    roll:
      check: grit
      threshold: 0
    do: []
",
    );
    support::stamp_pack_rev(&pack);
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    pack
}

fn lark_roll(pack: &Path, extra_first: bool) -> (String, i64) {
    let mut intents = Vec::new();
    if extra_first {
        intents.push(intent("briar-strike", "briar", "strike", Some("lark")));
    }
    intents.push(intent("lark-grit", "lark", "grit", None));
    let (_state, rolls, _round) = play_queued(pack, &[], intents);
    let rolled = rolls.into_iter().find_map(|report| match report {
        MoveReport::Rolled {
            actor_id,
            roll: out,
        } if actor_id == *"lark" => Some((
            out.faces
                .iter()
                .map(|face| face.to_string())
                .collect::<Vec<_>>()
                .join(","),
            out.total,
        )),
        _ => None,
    });
    rolled.unwrap_or_else(|| panic!("Lark did not roll"))
}

/// The GM queues `intents`, then one round plays. Any `seated` players are at the table for it.
fn play_queued(
    pack: &Path,
    seated: &[&str],
    intents: Vec<spookery_protocol::QueueArgs>,
) -> (spookery_engine::State, Vec<MoveReport>, Value) {
    let lock = spookery_engine::lock_pack(pack).unwrap();
    let mut world =
        spookery_engine::World::resume(pack, lock).unwrap_or_else(|err| panic!("open: {err}"));
    let presence = spookery_engine::Presence {
        players: u32::try_from(seated.len()).unwrap(),
        actors_present: seated.iter().map(|id| (*id).into()).collect(),
    };
    let wall = world.state().last_round_at.clone();
    for intent in intents {
        world
            .queue_gm_intent(&spookery_engine::Presence::default(), intent, || {
                panic!("the GM chose the id")
            })
            .unwrap_or_else(|code| panic!("queue: {code}"));
    }
    let output = world
        .play(&presence, &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
    let round = read_round(pack, "round-0001");
    let state = world.state().clone();
    (state, output.reports, round)
}

/// The GM's `queue` command: `actor_id` plays `verb` at `target`, under the id `id`.
fn intent(
    id: &str,
    actor_id: &str,
    verb: &str,
    target: Option<&str>,
) -> spookery_protocol::QueueArgs {
    spookery_protocol::QueueArgs {
        id: Some(id.to_string()),
        round_index: None,
        actor_id: Some(actor_id.to_string()),
        move_name: Some(verb.to_string()),
        target: target.map(str::to_string),
        message_to: None,
        rationale_code: "test".to_string(),
    }
}

fn make_mira_ineligible(pack: &Path) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace("  eligible: true\n", "  eligible: false\n");
    assert_ne!(text, next, "mira was already ineligible");
    fs::write(&path, next).unwrap();
    support::stamp_pack_rev(pack);
}

#[given(
    regex = r#"^the docks pack allows (\d+) state keys per round and a row "brace" that adds (\d+) to grit through a modifier$"#
)]
fn the_docks_pack_allows_keys_and_braces(world: &mut World, limit: u64, add: i64) {
    let pack = support::copy_pack("oneshot-docks", "key-limit-brace");
    set_budget(&pack, "max_state_keys_mutated", limit);
    fs::write(
        pack.join("rules/modifiers.yaml"),
        "schema: modifiers\nmodifiers:\n  - id: brace\n    meter: grit\n",
    )
    .unwrap();
    support::append(
        &pack.join("rules/verbs.yaml"),
        &format!(
            "
  - id: brace
    when: {{ verb: brace }}
    do:
      - modifier: {{ to: self, id: brace, add: {add}, ticks: 10 }}
"
        ),
    );
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

/// A cast member the save does not list, who carries the flag `marked`, and a row that sets it.
#[given(
    expr = "the docks pack has a cast member nell the save does not list and a row {string} that marks her"
)]
fn a_cast_member_the_save_does_not_list(world: &mut World, row: String) {
    let pack = support::copy_pack("oneshot-docks", "unlisted-actor");
    fs::write(
        pack.join("cast/pc-nell.md"),
        "---\nschema: cast\nid: nell\nkind: pc_template\ndisplay_name: Nell\nidle:\n  eligible: false\nflags:\n    - marked\n---\nNell.\n",
    )
    .unwrap();
    support::append(
        &pack.join("rules/verbs.yaml"),
        &format!(
            "
  - id: {row}
    when: {{ verb: {row} }}
    do:
      - flag: {{ of: nell, key: marked, value: true }}
"
        ),
    );
    make_mira_ineligible(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Rowan plays {word}")]
fn rowan_plays(world: &mut World, verb: String) {
    let pack = world.pack().to_path_buf();
    let lock = spookery_engine::lock_pack(&pack).unwrap();
    let mut opened = spookery_engine::World::resume(&pack, lock).unwrap();
    let wall = opened.state().last_round_at.clone();
    let nobody = spookery_engine::Presence::default();
    opened
        .queue_gm_intent(
            &nobody,
            intent(&format!("rowan-{verb}"), "rowan", &verb, None),
            || panic!("the GM chose the id"),
        )
        .unwrap();
    let output = opened.play(&nobody, &wall).unwrap();
    world.summary_lines = output.changed_keys.clone();
    world.scratch = read_round(&pack, "round-0001").to_string();
}

#[then(expr = "Rowan's {word} is refused {word}")]
fn rowans_move_is_refused(world: &mut World, verb: String, code: String) {
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    let refused = round["refused"].as_array().unwrap();
    assert_eq!(refused.len(), 1, "{round}");
    assert_eq!(
        refused[0]["id"],
        format!("rowan-{verb}").as_str(),
        "{round}"
    );
    assert_eq!(refused[0]["code"], code.as_str(), "{round}");
}

#[then(expr = "Rowan's brace lands, and the GM sees both {string} and {string} change")]
fn rowans_brace_lands(world: &mut World, modifier: String, meter: String) {
    let round: Value = serde_json::from_str(&world.scratch).unwrap();
    assert_eq!(round["refused"].as_array().unwrap().len(), 0, "{round}");
    for key in [modifier, meter] {
        assert!(
            world.summary_lines.contains(&key),
            "{key}: {:?}",
            world.summary_lines
        );
    }
}
