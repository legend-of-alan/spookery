use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::testing::{faces, RollKey};
use spookery_engine::MoveReport;
use spookery_engine::{Intent, IntentArgs, Source, Target, ViewerId};

use crate::World;
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

#[when(expr = "the same round is replayed with an extra unrelated NPC roll queued first")]
fn the_same_round_is_replayed_with_an_extra_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let (faces, total) = lark_roll(&pack, true);
    world.player_added = format!("{faces}|{total}");
}

#[then(expr = "Lark's faces and total are identical")]
fn larks_faces_and_total_are_identical(world: &mut World) {
    assert_eq!(world.player_added, world.scratch);
}

#[given(expr = "ten rolling intents in one round")]
fn ten_rolling_intents_in_one_round(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "ten-dice");
    let intents = (0..10)
        .map(|index| {
            queued(
                &format!("grit-{index}"),
                player("pc_rowan"),
                "grit",
                Target::Empty,
            )
        })
        .collect();
    let (_now, rolls, tick) = play_queued(&pack, intents);
    world.scratch = tick.to_string();
    world.player_added = rolls.len().to_string();
    world.pack = Some(pack);
}

#[then(expr = "all ten resolve and none is refused NotInSpec")]
fn all_ten_resolve_and_none_is_refused_not_in_spec(world: &mut World) {
    assert_eq!(world.player_added, "10");
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected.iter().all(|item| item["code"] != "NOT_IN_SPEC"),
        "{rejected:?}"
    );
}

#[given(expr = "a played d20 fight")]
fn a_played_d20_fight(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "d20-fight");
    let intents = vec![queued(
        "strike-lark",
        player("pc_lark"),
        "strike",
        Target::Actor("npc_briar".to_string()),
    )];
    let (now, _rolls, _tick) = play_queued(&pack, intents);
    world.now_bytes = spookery_engine::emit_save(&now).unwrap().into_bytes();
    world.pack = Some(pack);
}

#[then(expr = "spook verify passes and re-derives every roll")]
fn spook_verify_passes_and_re_derives_every_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    spookery_engine::verify_pack(pack, None).expect("verify");
    let report = spookery_engine::testing::from_journal(pack, None, None)
        .unwrap_or_else(|err| panic!("{err:?}"));
    let line = spookery_engine::testing::journal::read_record_line(pack, "round-0001")
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let tick: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        spookery_engine::testing::journal::hash_now(&report.now_yaml),
        tick["now_hash"].as_str().unwrap()
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
          who: place:docks_shop
          id: lamp_oil
          add: {add}
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: -{remove}
"
        ),
    );
    support::stamp_pack_rev(&pack);
    quiet_mira(&pack);
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
      target: 0
    do:
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: -{remove}
"
        ),
    );
    support::stamp_pack_rev(&pack);
    quiet_mira(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Mira plays {word}")]
fn mira_plays(world: &mut World, move_name: String) {
    let pack = world.pack.as_ref().unwrap().clone();
    let id = format!("{move_name}-1");
    let (now, rolls, tick) = play_queued(
        &pack,
        vec![queued(&id, npc("mira"), &move_name, Target::Empty)],
    );
    let oil = now
        .places
        .get("docks_shop")
        .and_then(|place| place.items.get("lamp_oil"))
        .copied()
        .map(i64::from)
        .unwrap_or(-1);
    world.now_bytes = oil.to_string().into_bytes();
    world.player_added = id;
    world.scratch = tick.to_string();
    world.mark = rolls
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
    assert_eq!(world.mark, 0, "a refused move reported a roll");
}

#[given(expr = "two kernel waits queued for Mira")]
fn two_kernel_waits_queued_for_mira(world: &mut World) {
    world.pack = Some(support::copy_pack("oneshot-docks", "second-wait"));
}

#[when(expr = "those waits are played")]
fn those_waits_are_played(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let mut first = queued("wait-1", npc("mira"), "wait", Target::Empty);
    let mut second = queued("wait-2", npc("mira"), "wait", Target::Empty);
    first.source = Source::Kernel;
    second.source = Source::Kernel;
    let (now, _rolls, tick) = play_queued(&pack, vec![first, second]);
    world.scratch = tick.to_string();
    world.now_bytes = spookery_engine::emit_save(&now).unwrap().into_bytes();
}

#[then(expr = "only the first wait lands and the second is INELIGIBLE")]
fn only_the_first_wait_lands(world: &mut World) {
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        !rejected.iter().any(|item| item["id"] == "wait-1"),
        "{tick}"
    );
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == "wait-2" && item["code"] == "INELIGIBLE"),
        "{tick}"
    );
    let now = spookery_engine::load_save(std::str::from_utf8(&world.now_bytes).unwrap()).unwrap();
    assert_eq!(
        now.actors
            .get("mira")
            .map(|actor| actor.act().last_act_round),
        Some(1)
    );
}

#[when(expr = "Lark and Briar strike each other")]
fn lark_and_briar_strike_each_other(world: &mut World) {
    let pack = support::copy_pack("sample-d20", "mutual-strike");
    let (_now, rolls, _tick) = play_queued(
        &pack,
        vec![
            queued(
                "lark-strike",
                player("pc_lark"),
                "strike",
                Target::Actor("npc_briar".to_string()),
            ),
            queued(
                "briar-strike",
                npc("npc_briar"),
                "strike",
                Target::Actor("pc_lark".to_string()),
            ),
        ],
    );
    world.pack = Some(pack);
    world.scratch = rolls
        .iter()
        .filter_map(|report| match report {
            MoveReport::Rolled { actor_id, out } => Some(format!(
                "{actor_id}={}:{}:{}",
                out.faces
                    .iter()
                    .map(|face| face.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                out.total,
                out.target
            )),
            MoveReport::Refused { .. } => None,
        })
        .collect::<Vec<_>>()
        .join(";");
}

#[then(expr = "each strike is scored on the other's own roll")]
fn each_strike_is_scored_on_the_others_own_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let lark = strike_line(&world.scratch, "pc_lark");
    let briar = strike_line(&world.scratch, "npc_briar");
    let lark_once = indexed_roll(pack, "pc_lark", 0);
    let lark_again = indexed_roll(pack, "pc_lark", 1);
    let briar_once = indexed_roll(pack, "npc_briar", 0);
    let briar_again = indexed_roll(pack, "npc_briar", 1);
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
    let tick_path = pack.join("rules/tick.yaml");
    let text = fs::read_to_string(&tick_path).unwrap();
    let next = text.replacen("max_effects_per_intent: 4", "max_effects_per_intent: 1", 1);
    assert_ne!(next, text, "cap was not lowered");
    fs::write(&tick_path, next).unwrap();
    support::append(
        &pack.join("rules/verbs.yaml"),
        "
  - id: two_stock
    when:
      verb: two_stock
    do:
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: 1
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: 1
",
    );
    support::stamp_pack_rev(&pack);
    quiet_mira(&pack);
    let (now, _rolls, tick) = play_queued(
        &pack,
        vec![queued("two-1", npc("mira"), "two_stock", Target::Empty)],
    );
    world.now_bytes = format!(
        "{}",
        now.places
            .get("docks_shop")
            .and_then(|place| place.items.get("lamp_oil"))
            .copied()
            .map(i64::from)
            .unwrap_or(-1)
    )
    .into_bytes();
    world.scratch = tick.to_string();
    world.pack = Some(pack);
}

#[then(expr = "no effect from that row applies and the tick log names the refusal")]
fn no_effect_from_that_row_applies(world: &mut World) {
    assert_eq!(String::from_utf8_lossy(&world.now_bytes), "4");
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    assert!(tick["effects"].as_array().unwrap().is_empty(), "{tick}");
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == "two-1" && item["code"] == "BUDGET"),
        "{tick}"
    );
}

#[given(
    expr = "a docks pack where Aldric's cast file holds a ten-minute verb \"ask\" that lowers the target's rest"
)]
fn a_docks_pack_where_aldric_holds_a_timed_verb(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "timed-target");
    let path = pack.join("cast/aldric.md");
    let text = fs::read_to_string(&path).unwrap();
    let rule = "  eligible: false
rules:
  - id: ask
    when: { verb: ask, target: actor }
    cost: { time: 10m }
    do: [meter: { who: target, key: rest, add: -1 }]
---";
    let next = text.replacen("  eligible: false\n---", rule, 1);
    assert_ne!(next, text, "aldric's front matter moved");
    fs::write(&path, next).unwrap();
    support::stamp_pack_rev(&pack);
    quiet_mira(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Rowan asks Aldric and the table plays until the action finishes")]
fn rowan_asks_aldric_until_it_finishes(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut table = spookery_engine::World::resume(&pack).unwrap_or_else(|err| panic!("{err}"));
    table.set_presence(support::Presence {
        humans: 1,
        actors_present: vec!["pc_rowan".to_string()],
    });
    table
        .queue_player_move("pc_rowan", "ask", Some("aldric"))
        .unwrap_or_else(|refusal| panic!("ask aldric: {refusal:?}"));
    let wall = table.now().last_tick_at.clone();
    for round in 1..=6 {
        let output = table.play(&wall);
        output.unwrap_or_else(|err| panic!("round {round}: {err}"));
        let now = table.now();
        if !now.actors["pc_rowan"].is_busy(now.world_time) {
            break;
        }
        assert!(round < 6, "Rowan is still busy after {round} rounds");
    }
    let now = table.now();
    world.mark =
        usize::try_from(now.meter_value(table.canon(), "aldric", "rest").unwrap()).unwrap();
}

#[then(expr = "Aldric's rest is {int}")]
fn aldrics_rest_is(world: &mut World, rest: usize) {
    assert_eq!(world.mark, rest);
}

fn assert_refusal(world: &World, oil: i64, reason: &str) {
    assert_eq!(String::from_utf8_lossy(&world.now_bytes), oil.to_string());
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    assert!(tick["effects"].as_array().unwrap().is_empty(), "{tick}");
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == world.player_added && item["code"] == reason),
        "{tick}"
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
    let world = spookery_engine::World::resume(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let actor = world
        .now()
        .actors
        .get(actor_id)
        .unwrap_or_else(|| panic!("missing {actor_id}"));
    let check = world.canon().checks.get("strike").unwrap();
    let def = world.canon().meters.get(&check.stat).unwrap();
    let live = actor.meters().get(&check.stat).unwrap();
    let stat = (live.base + live.offset).clamp(def.min, def.max);
    let rolled = faces(
        &RollKey {
            world_id: world.now().shard_id.clone(),
            round_index: world.now().round_index,
            rule_id: "strike".to_string(),
            subject_id: actor_id.to_string(),
            roll_index,
        },
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
    start: 1
",
    );
    support::append(
        &pack.join("rules/checks.yaml"),
        "
  - id: grit
    dice:
      count: 2
      sides: 6
    stat: grit
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
      target: 0
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
        intents.push(queued(
            "briar-strike",
            npc("npc_briar"),
            "strike",
            Target::Actor("npc_oden".to_string()),
        ));
    }
    intents.push(queued(
        "lark-grit",
        player("pc_lark"),
        "grit",
        Target::Empty,
    ));
    let (_now, rolls, _tick) = play_queued(pack, intents);
    let rolled = rolls.into_iter().find_map(|report| match report {
        MoveReport::Rolled { actor_id, out } if actor_id == "pc_lark" => Some((
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

fn play_queued(
    pack: &Path,
    intents: Vec<Intent>,
) -> (spookery_engine::Now, Vec<MoveReport>, Value) {
    let mut world =
        spookery_engine::World::resume(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let wall = world.now().last_tick_at.clone();
    for intent in intents {
        world.queue(intent);
    }
    let output = world
        .play(&wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
    let tick = read_round(pack, "round-0001");
    let now = world.now().clone();
    (now, output.reports, tick)
}

fn player(id: &str) -> ViewerId {
    ViewerId::Player(id.to_string())
}

fn npc(id: &str) -> ViewerId {
    ViewerId::Npc(id.to_string())
}

fn queued(id: &str, actor: ViewerId, move_name: &str, target: Target) -> Intent {
    Intent {
        id: id.to_string(),
        round_index: 1,
        actor,
        source: Source::Player,
        move_name: move_name.to_string(),
        target,
        args: IntentArgs::default(),
        rationale_code: "test".to_string(),
    }
}

fn quiet_mira(pack: &Path) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace("  eligible: true\n", "  eligible: false\n");
    assert_ne!(text, next, "mira was already ineligible");
    fs::write(&path, next).unwrap();
    support::stamp_pack_rev(pack);
}
