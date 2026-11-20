use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use spookery_engine::testing::roll;
use spookery_engine::{component, GlobalId, MoveReport, Note};

use crate::{resolve_id, World};
use spookery_engine::round::Output;
use spookery_engine::testing::{self as support, act_of, place_of};

#[given(expr = "a round where Harbormouse rolls exert")]
fn a_round_where_harbormouse_rolls_exert(world: &mut World) {
    let clean = support::copy_pack("docks", "harbormouse-exert");
    let played = support::fresh_dir("harbormouse-alone");
    support::copy_dir(&clean, &played);
    let (faces, total) = harbormouse_roll(&played, false);
    let _ = fs::remove_dir_all(played);
    world.scratch = format!("{faces}|{total}");
    world.pack = Some(clean);
}

#[when(expr = "the same round is played again with an extra unrelated NPC roll queued first")]
fn the_same_round_is_played_again_with_an_extra_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let (faces, total) = harbormouse_roll(&pack, true);
    world.intent_id = format!("{faces}|{total}");
}

#[then(expr = "Harbormouse's faces and total are identical")]
fn harbormouses_faces_and_total_are_identical(world: &mut World) {
    assert_eq!(world.intent_id, world.scratch);
}

#[given(expr = "ten rolling intents in one round")]
fn ten_rolling_intents_in_one_round(world: &mut World) {
    let pack = support::copy_pack("docks", "ten-dice");
    let intents = (0..10)
        .map(|index| intent(&format!("exert-{index}"), "rowan", "exert", None))
        .collect();
    let (_state, output) = play_queued(&pack, &[], intents);
    world.scratch = list_refusals(&output.reports);
    world.count = count_rolls(&output.reports, "rowan");
    world.pack = Some(pack);
}

#[then(expr = "all ten roll and none is refused")]
fn all_ten_roll_and_none_is_refused(world: &mut World) {
    assert_eq!(world.scratch, "");
    assert_eq!(world.count, 10);
}

#[given(expr = "a played d20 strike")]
fn a_played_d20_strike(world: &mut World) {
    let pack = support::copy_pack("docks", "d20-strike");
    let intents = vec![intent(
        "strike-harbormouse",
        "harbormouse",
        "strike",
        Some("briar"),
    )];
    let (ecs_world, _output) = play_queued(&pack, &[], intents);
    world.oil = spookery_engine::emit_save(&ecs_world).unwrap().into_bytes();
    world.pack = Some(pack);
}

#[then(expr = "spook verify passes and re-derives every roll")]
fn spook_verify_passes_and_re_derives_every_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    spookery_engine::verify_pack(pack, None).expect("verify");
}

#[given(expr = "two fresh copies of the docks pack as shipped")]
fn two_fresh_copies_of_the_docks_pack_as_shipped(world: &mut World) {
    let shipped = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/packs/docks");
    for label in ["fresh-a", "fresh-b"] {
        let pack = support::fresh_dir(label);
        support::copy_dir(&shipped, &pack);
        world.copies.push(pack);
    }
}

#[when(expr = "one round is played on each")]
fn one_round_is_played_on_each(world: &mut World) {
    for pack in &world.copies {
        play_queued(pack, &[], vec![intent("exert", "rowan", "exert", None)]);
    }
}

#[then(expr = "the two saves hold different seeds")]
fn the_two_saves_hold_different_seeds(world: &mut World) {
    let seeds: Vec<String> = world.copies.iter().map(|pack| saved_seed(pack)).collect();
    assert!(seeds.iter().all(|seed| !seed.is_empty()), "{seeds:?}");
    assert_ne!(seeds[0], seeds[1]);
}

#[then(expr = "spook verify passes on each")]
fn spook_verify_passes_on_each(world: &mut World) {
    for pack in &world.copies {
        spookery_engine::verify_pack(pack, None)
            .unwrap_or_else(|err| panic!("verify {}: {err:?}", pack.display()));
    }
}

fn saved_seed(pack: &Path) -> String {
    let text = fs::read_to_string(pack.join("state/save.yaml")).unwrap();
    spookery_engine::parse_save(&text).unwrap().seed
}

#[given(expr = "a docks row overreach that adds {int} lamp oil and then removes {int}")]
fn a_docks_row_overreach(world: &mut World, add: i64, remove: i64) {
    let pack = support::copy_pack("docks", "rollback");
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

#[given(expr = "a docks row spilled that rolls exert and then removes {int} lamp oil")]
fn a_docks_row_spilled(world: &mut World, remove: i64) {
    let pack = support::copy_pack("docks", "spilled");
    support::append(
        &pack.join("rules/verbs.yaml"),
        &format!(
            "
  - id: spilled
    when:
      verb: spilled
    check:
      total: 2d6 + self.vigor
      success_at: 0
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

#[then(expr = "lamp oil stays {int}")]
fn lamp_oil_stays(world: &mut World, oil: i64) {
    assert_eq!(String::from_utf8_lossy(&world.oil), oil.to_string());
}

#[then(expr = "lamp oil stays {int} and no roll is reported")]
fn lamp_oil_stays_and_no_roll_is_reported(world: &mut World, oil: i64) {
    lamp_oil_stays(world, oil);
    assert_eq!(world.count, 0, "a refused move reported a roll");
}

#[given(expr = "the docks pack, where Mira's door-note habit is due")]
fn the_docks_pack_where_miras_habit_is_due(world: &mut World) {
    let pack = support::copy_pack("docks", "habit-ineligible");
    let opened = spookery_engine::Session::peek(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    let view = opened
        .perceive(&spookery_engine::Presence::default(), "mira")
        .unwrap_or_else(|err| panic!("perceive: {err}"));
    assert!(view.habits.iter().any(|habit| habit == "leave_door_note"));
    world.pack = Some(pack);
}

#[when(expr = "the GM queues mira wait and an empty round plays")]
fn the_gm_queues_mira_wait(world: &mut World) {
    let wait = intent("gm-wait", "mira", "wait", None);
    play_queued(world.pack(), &[], vec![wait]);
}

#[then(expr = "the wait lands, and Mira's habit does not run because she already acted this round")]
fn the_wait_lands_and_miras_habit_does_not_run(world: &mut World) {
    let ecs_world = support::world_of(world.pack());
    assert_eq!(act_of(&ecs_world, "mira").last_act_round, 1);
    let notes = ecs_world.globals().get::<Note>(&GlobalId);
    assert!(
        !notes.contains("note_mira_door"),
        "the door note was stored"
    );
}

#[when(expr = "Harbormouse and Briar strike each other")]
fn harbormouse_and_briar_strike_each_other(world: &mut World) {
    let pack = support::copy_pack("docks", "mutual-strike");
    let (_state, output) = play_queued(
        &pack,
        &[],
        vec![
            intent("harbormouse-strike", "harbormouse", "strike", Some("briar")),
            intent("briar-strike", "briar", "strike", Some("harbormouse")),
        ],
    );
    world.pack = Some(pack);
    world.scratch = (output.reports.iter())
        .filter_map(|report| match report {
            MoveReport::Rolled { actor_id, roll } => {
                Some(format!("{actor_id}={}:{}", roll.total, roll.success_at))
            }
            MoveReport::Refused { .. } => None,
        })
        .collect::<Vec<_>>()
        .join(";");
}

#[then(expr = "each strike rolls once against success_at {int}")]
fn each_strike_rolls_once_against_success_at(world: &mut World, expected: i64) {
    let pack = world.pack.as_ref().unwrap();
    assert_eq!(world.scratch.split(';').count(), 2, "{}", world.scratch);
    for actor_id in ["harbormouse", "briar"] {
        let (total, success_at) = strike_line(&world.scratch, actor_id);
        assert_eq!(
            total,
            derive_first_strike(pack, actor_id),
            "{actor_id} did not roll its own first key"
        );
        assert_eq!(success_at, expected, "{actor_id}'s strike took another bar");
    }
}

#[given(expr = "max_effects_per_intent is 1 and a row with 2 effects")]
fn max_effects_per_intent_is_1_and_a_row_with_2_effects(world: &mut World) {
    let pack = support::copy_pack("docks", "effect-cap");
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
    let (ecs_world, _output) =
        play_queued(&pack, &[], vec![intent("two-1", "mira", "two_stock", None)]);
    world.oil = format!(
        "{}",
        ecs_world
            .places()
            .get::<component::Item>(&"docks_shop".into())
            .get("lamp_oil")
            .copied()
            .map(i64::from)
            .unwrap_or(-1)
    )
    .into_bytes();
    world.pack = Some(pack);
}

#[then(expr = "no effect from that row applies")]
fn no_effect_from_that_row_applies(world: &mut World) {
    assert_eq!(String::from_utf8_lossy(&world.oil), "4");
}

#[given(
    expr = "a docks pack where Aldric's cast file holds a ten-tick verb \"ask\" that lowers the target's rest"
)]
fn a_docks_pack_where_aldric_holds_a_timed_verb(world: &mut World) {
    let pack = support::copy_pack("docks", "timed-target");
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
    let mut session =
        spookery_engine::Session::resume(&pack, lock).unwrap_or_else(|e| panic!("{e}"));
    let rowan = spookery_engine::Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    };
    session
        .queue_player_move(&rowan, &"rowan".into(), "ask", Some("aldric"))
        .unwrap_or_else(|refusal| panic!("ask aldric: {refusal:?}"));
    let wall = session.world().last_round_at.clone();
    for round in 1..=6 {
        let output = session.play(&rowan, &wall);
        output.unwrap_or_else(|err| panic!("round {round}: {err}"));
        let ecs_world = session.world();
        if !support::is_busy(ecs_world, "rowan") {
            break;
        }
        assert!(round < 6, "Rowan is still busy after {round} rounds");
    }
}

#[then(regex = r"^(?P<actor>\S+)'s rest is (?P<rest>\d+)$")]
fn rest_is(world: &mut World, actor: String, rest: i64) {
    let now = support::world_of(world.pack());
    let actor = spookery_engine::entity::Id::Actor(resolve_id(now.canon(), &actor).into());
    let held = now.entities().get::<component::Meter>(&actor).value("rest");
    assert_eq!(held, Ok(rest), "{}", world.scratch);
}

#[given(
    expr = "a docks pack whose verbs hold a ten-tick verb \"haul\" that takes {int} lamp oil from the actor"
)]
fn a_docks_pack_with_a_timed_haul(world: &mut World, oil: i64) {
    let pack = support::copy_pack("docks", "timed-refused");
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
    let mut session = support::resume(&pack);
    let rowan = spookery_engine::Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    };
    session
        .queue_player_move(&rowan, &"rowan".into(), "haul", None)
        .unwrap_or_else(|refusal| panic!("haul: {refusal:?}"));
    let wall = session.world().last_round_at.clone();
    world.scratch.clear();
    for round in 1..=6 {
        match session.play(&rowan, &wall) {
            Ok(output) => world.seen.push_str(&list_refusals(&output.reports)),
            Err(err) => {
                world.scratch = format!("round {round} failed: {err}");
                return;
            }
        }
        let ecs_world = session.world();
        if !support::is_busy(ecs_world, "rowan") {
            break;
        }
    }
}

#[then(
    expr = "the finishing round commits with Rowan's haul refused MISSING_ITEM, Rowan is no longer busy, and spook verify passes"
)]
fn the_refused_finish_commits(world: &mut World) {
    assert!(world.scratch.is_empty(), "{}", world.scratch);
    assert_eq!(world.seen, "rowan MISSING_ITEM");
    let pack = world.pack();
    let saved = spookery_engine::Session::peek(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let ecs_world = saved.world();
    assert!(!support::is_busy(ecs_world, "rowan"));
    spookery_engine::verify_pack(pack, None).unwrap_or_else(|err| panic!("verify: {err}"));
}

#[given(expr = "a docks pack whose \"pick\" rule has a success_at that divides by zero")]
fn a_docks_pack_with_a_broken_pick(world: &mut World) {
    let pack = support::copy_pack("docks", "broken-success-at");
    support::append(
        &pack.join("rules/verbs.yaml"),
        "
  - id: pick
    when: { verb: pick }
    check: { total: 2d6 + self.vigor, success_at: \"10 / (self.vigor - 1)\" }
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
    let (_state, output) = rowan_walks(world.pack(), vec![pick]);
    world.scratch = list_refusals(&output.reports);
}

#[then(expr = "Wren's pick is refused {word} and Rowan is on the street")]
fn wrens_pick_is_refused(world: &mut World, code: String) {
    assert_eq!(world.scratch, format!("wren {code}"));
    let saved = spookery_engine::Session::peek(world.pack()).unwrap();
    assert_eq!(place_of(saved.world(), "rowan"), "docks_street");
}

#[then(expr = "the next round plays and spook verify passes")]
fn the_next_round_plays_and_verify_passes(world: &mut World) {
    let pack = world.pack();
    let lock = spookery_engine::lock_pack(pack).unwrap();
    let mut table =
        spookery_engine::Session::resume(pack, lock).unwrap_or_else(|err| panic!("open: {err}"));
    let wall = table.world().last_round_at.clone();
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
    assert_eq!(table.world().round_index, 2);
    drop(table);
    spookery_engine::verify_pack(pack, None).unwrap_or_else(|err| panic!("verify: {err}"));
}

#[given(
    expr = "a docks pack where the shop's \"rummage\" costs {int} rest and rolls against the actor's rest, and the shared \"rummage\" costs {int}"
)]
fn a_docks_pack_with_two_rummage_rules(world: &mut World, shop: i64, shared: i64) {
    let pack = support::copy_pack("docks", "rummage");
    let rule = format!(
        "  - id: rummage
    when: {{ verb: rummage }}
    cost: {{ meters: {{ rest: {shop} }} }}
    check: {{ total: 2d6 + self.vigor, success_at: self.rest }}
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

#[given(expr = "the walk to the street costs {int} rest")]
fn the_walk_to_the_street_costs_rest(world: &mut World, rest: i64) {
    let path = world.pack().join("world/map.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let line = "  - path: docks_shop <-> docks_street\n";
    let cost = format!("    cost: {{ meters: {{ rest: {rest} }} }}\n");
    let split = format!(
        "  - path: docks_shop -> docks_street\n{cost}  - path: docks_street -> docks_shop\n"
    );
    let next = text.replacen(line, &split, 1);
    assert_ne!(next, text, "the shop has no two-way street path");
    fs::write(&path, next).unwrap();
    support::stamp_pack_rev(world.pack());
}

#[when(expr = "Rowan walks to the street and rummages in the same round")]
fn rowan_walks_and_rummages(world: &mut World) {
    let rummage = intent("rummage", "rowan", "rummage", None);
    let (_state, output) = rowan_walks(world.pack(), vec![rummage]);
    world.seen = (output.reports.iter())
        .filter_map(|report| match report {
            MoveReport::Rolled { actor_id, roll } if actor_id == "rowan" => {
                Some(roll.success_at.to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(", ");
}

#[then(expr = "the GM sees Rowan's check against a bar of {int}")]
fn the_gm_sees_rowans_check_against_a_bar_of(world: &mut World, bar: i64) {
    assert_eq!(world.seen, bar.to_string());
}

#[given(
    expr = "max_effects_per_intent is 1 and a street rule that sets 2 flags when Rowan arrives"
)]
fn a_street_rule_over_the_cap(world: &mut World) {
    let pack = support::copy_pack("docks", "trigger-cap");
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
    let (ecs_world, _output) = rowan_walks(world.pack(), Vec::new());
    let rowan = "rowan".into();
    let flags = ecs_world.actors().get::<component::Flag>(&rowan);
    world.count = flags.iter().count();
}

#[then(expr = "neither flag is set")]
fn neither_flag_is_set(world: &mut World) {
    assert_eq!(world.count, 0, "flags were set");
}

/// Only the shop's side of the stair door opens with the key, so this opens the rooms' side.
#[given(expr = "the stair door stands open on Mira's side")]
fn the_stair_door_stands_open_on_miras_side(world: &mut World) {
    support::edit_save(world.pack(), |now| {
        let stairs = spookery_engine::ExitId {
            place: "mira_rooms".into(),
            exit: "stairs".to_string(),
        };
        now.places_mut().set_exit_open(&stairs, true).unwrap();
    });
}

#[when(expr = "Bram and Aldric wait and then Mira travels through the {string} exit")]
fn two_wait_and_mira_travels(world: &mut World, exit: String) {
    let intents = vec![
        intent("wait-bram", "bram", "wait", None),
        intent("wait-aldric", "aldric", "wait", None),
        intent("walk-mira", "mira", "travel", Some(&exit)),
    ];
    play_queued(world.pack(), &["rowan"], intents);
}

/// The round has room for one intent, so only the one picked first acts.
#[then(expr = "Mira's travel is picked ahead of both waits")]
fn miras_travel_is_picked_first(world: &mut World) {
    let now = support::world_of(world.pack());
    for actor in ["bram", "aldric"] {
        assert_eq!(
            act_of(&now, actor).last_act_round,
            0,
            "{actor}'s wait landed"
        );
    }
}

#[then(expr = "Mira has visited the Docks Shop")]
fn mira_has_visited_the_shop(world: &mut World) {
    let saved = spookery_engine::Session::peek(world.pack()).unwrap();
    let (ecs_world, mira) = (saved.world(), "mira".into());
    assert_eq!(place_of(ecs_world, "mira"), "docks_shop");
    let visits = ecs_world.actors().get::<component::Visit>(&mira);
    let visited: Vec<&str> = visits.iter().map(|(place, _)| place).collect();
    assert!(visits.contains("docks_shop"), "{visited:?}");
}

#[given(expr = "the docks pack allows {int} intent per round")]
fn the_docks_pack_allows_intents(world: &mut World, limit: u64) {
    let pack = support::copy_pack("docks", "intent-limit");
    set_budget(&pack, "max_intents_applied", limit);
    support::stamp_pack_rev(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Harbormouse waits and Wren waits in the same round")]
fn harbormouse_and_wren_wait(world: &mut World) {
    let intents = vec![
        intent("harbormouse-wait", "harbormouse", "wait", None),
        intent("wren-wait", "wren", "wait", None),
    ];
    let (_state, output) = play_queued(world.pack(), &[], intents);
    world.scratch = list_refusals(&output.reports);
}

#[then(expr = "Wren's wait is refused BUDGET and nothing is left queued")]
fn wrens_wait_is_refused_budget(world: &mut World) {
    assert_budget_refusals(world, "wren BUDGET");
}

#[given(
    expr = "the docks pack allows {int} state key per round and a row \"overset\" that sets 2 flags"
)]
fn the_docks_pack_allows_keys(world: &mut World, limit: u64) {
    let pack = support::copy_pack("docks", "key-limit");
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
    let (_state, output) = play_queued(world.pack(), &[], intents);
    world.scratch = list_refusals(&output.reports);
}

#[then(expr = "Rowan's overset and Wren's wait are both refused BUDGET and nothing is left queued")]
fn rowans_overset_and_wrens_wait_are_refused_budget(world: &mut World) {
    assert_budget_refusals(world, "rowan BUDGET, wren BUDGET");
}

/// The round's refusals are `expected`, and the save keeps no queue.
fn assert_budget_refusals(world: &World, expected: &str) {
    assert_eq!(world.scratch, expected);
    let saved = spookery_engine::Session::peek(world.pack()).unwrap();
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
) -> (spookery_engine::ecs::World, Output) {
    let mut intents = vec![intent("walk", "rowan", "travel", Some("docks_street"))];
    intents.extend(then);
    let (world, output) = play_queued(pack, &["rowan"], intents);
    assert_eq!(place_of(&world, "rowan"), "docks_street");
    (world, output)
}

/// Give `place`, which has no rules, its own `rules:` list at the end of its frontmatter.
pub(crate) fn add_place_rules(pack: &Path, place: &str, rules: &str) {
    let path = pack.join(format!("world/places/{place}.md"));
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("\nrules:"), "{place} has rules already");
    let next = text.replacen("\n---\n", &format!("\nrules:\n{rules}\n---\n"), 1);
    assert_ne!(next, text, "{place} has no closing fence");
    fs::write(&path, next).unwrap();
}

/// Each refused move in `reports` as `actor CODE`, joined by commas.
pub(crate) fn list_refusals<'r>(reports: impl IntoIterator<Item = &'r MoveReport>) -> String {
    let refused = reports.into_iter().filter_map(|report| match report {
        MoveReport::Refused { actor_id, refusal } => Some(format!("{actor_id} {}", refusal.code)),
        MoveReport::Rolled { .. } => None,
    });
    refused.collect::<Vec<_>>().join(", ")
}

/// How many rolls `reports` holds for `actor`.
fn count_rolls(reports: &[MoveReport], actor: &str) -> usize {
    let rolled = |report: &&MoveReport| matches!(report, MoveReport::Rolled { actor_id, .. } if actor_id == actor);
    reports.iter().filter(rolled).count()
}

fn strike_line(scratch: &str, actor: &str) -> (i64, i64) {
    let prefix = format!("{actor}=");
    let line = scratch
        .split(';')
        .find(|item| item.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no roll for {actor} in {scratch}"));
    let rest = line.trim_start_matches(&prefix);
    let (total, success_at) = rest.split_once(':').unwrap();
    (total.parse().unwrap(), success_at.parse().unwrap())
}

/// The total of `actor_id`'s first strike key this round: the docks strike's `1d20 + self.might`.
fn derive_first_strike(pack: &Path, actor_id: &str) -> i64 {
    let session = spookery_engine::Session::peek(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let desc = session.canon().meters.get("might").unwrap();
    let actor = actor_id.into();
    let live = session
        .world()
        .actors()
        .get::<component::Meter>(&actor)
        .get("might")
        .unwrap_or_else(|| panic!("missing {actor_id}'s might"));
    let stat = (live.base + live.offset).clamp(desc.min, desc.max);
    let rolled = roll::faces(
        &roll::Key {
            seed: session.world().seed.clone(),
            round_index: session.world().round_index,
            rule_id: "verb:strike".to_string(),
            subject_id: Some(actor_id.into()),
            index: 0,
        },
        0,
        1,
        20,
    )
    .unwrap_or_else(|err| panic!("roll: {err}"));
    i64::from(rolled[0]) + stat
}

fn harbormouse_roll(pack: &Path, extra_first: bool) -> (String, i64) {
    let mut intents = Vec::new();
    if extra_first {
        intents.push(intent(
            "briar-strike",
            "briar",
            "strike",
            Some("harbormouse"),
        ));
    }
    intents.push(intent("harbormouse-exert", "harbormouse", "exert", None));
    let (_state, output) = play_queued(pack, &[], intents);
    let total = output.reports.into_iter().find_map(|report| match report {
        MoveReport::Rolled { actor_id, roll } if actor_id == *"harbormouse" => Some(roll.total),
        _ => None,
    });
    let total = total.unwrap_or_else(|| panic!("Harbormouse did not roll"));
    let exert_line = (output.dice.iter())
        .find(|line| line.starts_with("2d6 "))
        .unwrap_or_else(|| panic!("no exert dice line: {:?}", output.dice));
    (exert_line.to_string(), total)
}

/// The GM queues `intents`, then one round plays. Any `seated` players are at the table for it.
fn play_queued(
    pack: &Path,
    seated: &[&str],
    intents: Vec<spookery_protocol::QueueArgs>,
) -> (spookery_engine::ecs::World, Output) {
    let lock = spookery_engine::lock_pack(pack).unwrap();
    let mut session =
        spookery_engine::Session::resume(pack, lock).unwrap_or_else(|err| panic!("open: {err}"));
    let presence = spookery_engine::Presence {
        players: u32::try_from(seated.len()).unwrap(),
        actors_present: seated.iter().map(|id| (*id).into()).collect(),
    };
    let wall = session.world().last_round_at.clone();
    for intent in intents {
        session
            .queue_gm_intent(&spookery_engine::Presence::default(), intent, || {
                panic!("the GM chose the id")
            })
            .unwrap_or_else(|code| panic!("queue: {code}"));
    }
    let output = session
        .play(&presence, &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
    (session.world().clone(), output)
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
        move_id: Some(verb.to_string()),
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
    regex = r#"^the docks pack allows (\d+) state keys per round and a row "brace" that adds (\d+) to vigor through a modifier$"#
)]
fn the_docks_pack_allows_keys_and_braces(world: &mut World, limit: u64, add: i64) {
    let pack = support::copy_pack("docks", "key-limit-brace");
    set_budget(&pack, "max_state_keys_mutated", limit);
    fs::write(
        pack.join("rules/modifiers.yaml"),
        "schema: modifiers\nmodifiers:\n  - id: brace\n    meter: vigor\n",
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
    let pack = support::copy_pack("docks", "unlisted-actor");
    fs::write(
        pack.join("cast/pc-nell.md"),
        "---\nschema: cast\nid: nell\ncontrol_type: pc_template\nname: Nell\nidle:\n  eligible: false\nflags:\n    - marked\n---\nNell.\n",
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

/// The GM queues Rowan's `verb`. The scenario keeps the shop's lamp oil, the round's
/// refusals, Rowan's rolls and what the GM sees change.
#[when(expr = "Rowan plays {word}")]
fn rowan_plays(world: &mut World, verb: String) {
    let queued = intent(&format!("rowan-{verb}"), "rowan", &verb, None);
    let (ecs_world, output) = play_queued(world.pack(), &[], vec![queued]);
    let shop = "docks_shop".into();
    let items = ecs_world.places().get::<component::Item>(&shop);
    let oil = items.get("lamp_oil").copied().map_or(-1, i64::from);
    world.oil = oil.to_string().into_bytes();
    world.scratch = list_refusals(&output.reports);
    world.count = count_rolls(&output.reports, "rowan");
    world.summary_lines = output.changed_keys;
}

#[then(expr = "Rowan's move is refused {word}")]
fn rowans_move_is_refused(world: &mut World, code: String) {
    assert_eq!(world.scratch, format!("rowan {code}"));
}

#[then(expr = "Rowan's brace lands, and the GM sees both {string} and {string} change")]
fn rowans_brace_lands(world: &mut World, modifier: String, meter: String) {
    assert_eq!(world.scratch, "");
    for key in [modifier, meter] {
        assert!(
            world.summary_lines.contains(&key),
            "{key}: {:?}",
            world.summary_lines
        );
    }
}
