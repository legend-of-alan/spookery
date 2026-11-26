use spookery_engine::component;
use spookery_engine::testing::world_of;
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, copy_pack};
use crate::World;

use super::{common, host};

/// Docks exert is 2d6 plus a vigor of 1, so the total is 3..=13.
const NOTICE_ROWS: &str = "
  - id: notice_sure
    when:
      verb_id: notice_sure
    check:
      total: 2d6 + self.vigor
      success_at: 3
    success:
      - flag:
          of: self
          id: braced
          value: true
    miss:
      - flag:
          of: self
          id: slipped
          value: true
  - id: notice_hard
    when:
      verb_id: notice_hard
    check:
      total: 2d6 + self.vigor
      success_at: 14
    success:
      - flag:
          of: self
          id: braced
          value: true
    miss:
      - flag:
          of: self
          id: slipped
          value: true
";

const CONTESTED_MARK_ROW: &str = "
  - id: mark
    when:
      verb_id: mark
      target: actor
    let:
      attack: 2d6 + self.vigor
      dodge: 2d6 + target.vigor
    check:
      total: attack
      success_at: dodge
      partial_at: dodge - 4
    success:
      - flag:
          of: target
          id: marked
          value: true
    partial:
      - flag:
          of: target
          id: grazed
          value: true
    miss:
      - flag:
          of: target
          id: missed
          value: true
";

const MIRA_VIGOR: i64 = 100;

#[when(expr = "the player rolls exert")]
async fn the_player_rolls_exert(world: &mut World) {
    let (mark, added) = host::player_send(world, "exert", "\"kind\":").await;
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the player sees a check success")]
async fn the_player_sees_a_check_success(world: &mut World) {
    world.player_added = host::wait_player_from(
        world,
        world.mark,
        "{\"kind\":\"check\",\"outcome\":\"success\"}",
    )
    .await;
    assert!(
        !world.player_added.contains("\"dice\""),
        "{}",
        world.player_added
    );
    assert!(
        !world.player_added.contains("\"success\":"),
        "{}",
        world.player_added
    );
}

#[then(expr = "the GM sees the exert faces")]
async fn the_gm_sees_the_exert_faces(world: &mut World) {
    let text = host::wait_gm(world, "\"rolls\"").await;
    assert_roll(&read_committed(&text), "", 1, 0, "success");
}

#[then(expr = "the player sees a check result without faces")]
async fn the_player_sees_a_check_result_without_faces(world: &mut World) {
    let added = host::wait_player_from(world, world.mark, "{\"kind\":\"check\",\"outcome\":").await;
    assert!(
        added.contains("{\"kind\":\"check\",\"outcome\":\"success\"}")
            || added.contains("{\"kind\":\"check\",\"outcome\":\"partial\"}")
            || added.contains("{\"kind\":\"check\",\"outcome\":\"miss\"}"),
        "{added}"
    );
    assert!(!added.contains("\"success\":"), "{added}");
    assert!(!added.contains("\"dice\""), "{added}");
}

#[given(expr = "a docks table where Mira can notice")]
async fn a_docks_table_where_mira_can_notice(world: &mut World) {
    let pack = copy_pack("docks", "npc-notice");
    world.pack = Some(pack.clone());
    common::add_verb(&pack, NOTICE_ROWS);
    support::stamp_pack_rev(&pack);
    listen_and_seat_both(world, &pack).await;
    world.mark = host::player_mark(world);
}

#[given(expr = "a docks table where Mira's vigor contests a mark")]
async fn a_docks_table_where_miras_vigor_contests_a_mark(world: &mut World) {
    let pack = copy_pack("docks", "contested-mark");
    world.pack = Some(pack.clone());
    common::add_verb(&pack, CONTESTED_MARK_ROW);
    set_vigor(&pack, "mira", MIRA_VIGOR);
    super::common::edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
    });
    listen_and_seat_both(world, &pack).await;
}

#[when(expr = "the GM queues Mira's sure notice")]
async fn the_gm_queues_miras_sure_notice(world: &mut World) {
    queue_mira(world, "notice_sure").await;
}

#[when(expr = "the GM queues Mira's hard notice")]
async fn the_gm_queues_miras_hard_notice(world: &mut World) {
    queue_mira(world, "notice_hard").await;
}

#[when(expr = "an engine habit rolls a check for Mira")]
fn an_engine_habit_rolls_a_check_for_mira(world: &mut World) {
    let pack = copy_pack("docks", "npc-habit");
    world.pack = Some(pack.clone());
    insert_brace_habit(&pack);
    support::stamp_pack_rev(&pack);
    spookery_engine::play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
}

#[when(expr = "the player sends a line with a player prefix")]
async fn the_player_sends_a_line_with_a_player_prefix(world: &mut World) {
    let (mark, added) = host::player_send(world, "player:nell exert", "\"kind\":\"refused\"").await;
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "Mira is braced")]
fn mira_is_braced(world: &mut World) {
    assert_eq!(flags_of(world, "mira"), vec!["braced".to_string()]);
}

#[then(expr = "Mira slipped")]
fn mira_slipped(world: &mut World) {
    assert_eq!(flags_of(world, "mira"), vec!["slipped".to_string()]);
}

#[then(expr = "the mark missed Mira")]
fn the_mark_missed_mira(world: &mut World) {
    assert_eq!(flags_of(world, "mira"), vec!["missed".to_string()]);
}

#[then(expr = "the GM sees Mira's sure notice")]
async fn the_gm_sees_miras_sure_notice(world: &mut World) {
    assert_one_roll(world, 1, 3, "success").await;
}

#[then(expr = "the GM sees Mira's failed notice")]
async fn the_gm_sees_miras_failed_notice(world: &mut World) {
    assert_one_roll(world, 1, 14, "miss").await;
}

#[then(expr = "the player sees no check frame")]
async fn the_player_sees_no_check_frame(world: &mut World) {
    let added = host::wait_player_from(world, world.mark, "\"kind\":\"view\"").await;
    assert!(!added.contains("\"kind\":\"check\""), "{added}");
}

#[then(expr = "the player sees only their own check")]
async fn the_player_sees_only_their_own_check(world: &mut World) {
    let added = host::wait_player_from(
        world,
        world.mark,
        "{\"kind\":\"check\",\"outcome\":\"miss\"}",
    )
    .await;
    assert_eq!(added.matches("\"kind\":\"check\"").count(), 1, "{added}");
    assert!(!added.contains("\"dice\""), "{added}");
    assert!(!added.contains("\"success\":"), "{added}");
}

#[then(expr = "the GM sees one roll read from the attack and dodge dice lines")]
fn the_gm_sees_one_roll_from_attack_and_dodge(world: &mut World) {
    let body = host::read_reply(&world.stdout);
    let dice = body["dice"].as_array().unwrap_or_else(|| panic!("{body}"));
    assert_eq!(dice.len(), 2, "{body}");
    let (faces, _) = dice[1]
        .as_str()
        .and_then(|line| line.strip_prefix("dodge: 2d6 → "))
        .and_then(|rest| rest.split_once(" = "))
        .unwrap_or_else(|| panic!("{body}"));
    let face_sum = faces
        .split(", ")
        .map(str::parse::<i64>)
        .sum::<Result<i64, _>>()
        .unwrap_or_else(|err| panic!("{err}: {body}"));
    assert_roll(&body, "attack: ", 1, face_sum + MIRA_VIGOR, "miss");
}

#[then(expr = "play shows {int} successful rolls")]
async fn play_shows_successful_rolls(world: &mut World, rolls: usize) {
    let succeeded = |stdout: &str| stdout.matches("Your roll succeeded.").count() >= rolls;
    common::wait_play_until(world, succeeded, &format!("{rolls} successful rolls")).await;
}

#[then(expr = "{word}'s hp is {int}")]
async fn hp_is(world: &mut World, actor: String, expected: i64) {
    let pack = host::pack(world).to_path_buf();
    let id = spookery_engine::entity::Id::Actor(actor.as_str().into());
    host::poll_until(&format!("{actor} hp {expected}"), async || {
        let now = world_of(&pack);
        match now.entities().get::<component::Meter>(&id).value("hp") {
            Ok(hp) if hp == expected => Ok(()),
            got => Err(format!("hp {got:?}")),
        }
    })
    .await;
}

async fn listen_and_seat_both(world: &mut World, pack: &Path) {
    host::listen(world, pack).await;
    host::seat_gm(world).await;
    host::seat_rowan(world, pack).await;
}

async fn queue_mira(world: &mut World, move_id: &str) {
    let args = format!(r#"{{"actor":"mira","verb":"{move_id}"}}"#);
    let text = host::gm_tool_call(world, 4, "queue", &args).await;
    assert!(text.contains("\"id\""), "{text}");
    assert!(!text.contains("UNKNOWN_ID"), "{text}");
}

fn insert_brace_habit(pack: &Path) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read mira: {err}"));
    let next = text.replacen(
        "rules:\n  - id: leave_door_note\n",
        "rules:\n  - id: brace\n    when: habit\n    check:\n      total: 2d6 + self.vigor\n      success_at: 14\n    success:\n      - flag:\n          of: self\n          id: braced\n          value: true\n    miss:\n      - flag:\n          of: self\n          id: slipped\n          value: true\n  - id: leave_door_note\n",
        1,
    );
    assert_ne!(next, text, "mira habit was not inserted");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write mira: {err}"));
}

/// Set `actor`'s vigor to `base`, through the writer.
fn set_vigor(pack: &Path, actor: &str, base: i64) {
    support::edit_save(pack, |now| {
        support::edit_meter(now, actor, "vigor", |vigor| vigor.base = base);
    });
}

fn flags_of(world: &World, id: &str) -> Vec<String> {
    let now = world_of(host::pack(world));
    assert!(now.actors().contains(&id.into()), "{id} missing");
    let id = id.into();
    let flags = now.actors().get::<component::Flag>(&id);
    flags.iter().map(|(flag, ())| flag.to_string()).collect()
}

async fn assert_one_roll(world: &mut World, vigor: i64, success_at: i64, outcome: &str) {
    let text = host::wait_gm(world, "\"rolls\"").await;
    assert_roll(&read_committed(&text), "", vigor, success_at, outcome);
}

/// `body` holds one 2d6 roll: its first dice line reads `<label>2d6 → a, b = a+b + vigor`, and
/// the roll totals that line against `success_at`.
fn assert_roll(body: &Value, label: &str, vigor: i64, success_at: i64, outcome: &str) {
    let rolls = body["rolls"].as_array().unwrap_or_else(|| panic!("{body}"));
    assert_eq!(rolls.len(), 1, "{body}");
    let line = body["dice"][0].as_str().unwrap_or_else(|| panic!("{body}"));
    assert!(
        line.starts_with(&format!("{label}2d6 → ")) && line.ends_with(&format!(" + {vigor}")),
        "{body}"
    );
    assert_eq!(
        rolls[0]["total"].as_i64(),
        Some(dice_line_total(line)),
        "{body}"
    );
    assert_eq!(rolls[0]["success_at"].as_i64(), Some(success_at), "{body}");
    assert_eq!(rolls[0]["outcome"].as_str(), Some(outcome), "{body}");
    assert!(rolls[0].get("success").is_none(), "{body}");
}

/// What a dice line comes to: `2d6 → 5, 2 = 7 + 1` is 8.
fn dice_line_total(line: &str) -> i64 {
    let (_, sum) = line.split_once(" = ").unwrap_or_else(|| panic!("{line}"));
    sum.split(" + ")
        .map(|part| part.parse::<i64>().unwrap_or_else(|_| panic!("{line}")))
        .sum()
}

/// The `committed` body in a GM tool result: the first object whose `rolls` is not empty.
fn read_committed(text: &str) -> Value {
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(body) = find_committed(&value) {
            return body;
        }
    }
    panic!("tool result has no rolls\n{text}");
}

fn find_committed(value: &Value) -> Option<Value> {
    let rolls = value.get("rolls").and_then(Value::as_array);
    if rolls.is_some_and(|rolls| !rolls.is_empty()) {
        return Some(value.clone());
    }
    match value {
        Value::Object(map) => map.values().find_map(find_committed),
        Value::Array(items) => items.iter().find_map(find_committed),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

const PRESS_ROW: &str = "
  - id: press
    when:
      verb_id: press
      target: actor
    check:
      total: 1d1 + self.vigor
      success_at: 10
      partial_at: 7
    do:
      - flag:
          of: target
          id: touched
          value: true
    success:
      - flag:
          of: target
          id: clean
          value: true
    partial:
      - flag:
          of: target
          id: halfway
          value: true
    miss:
      - flag:
          of: target
          id: missed
          value: true
";

const NOTICE_PARTIAL_ROW: &str = "
  - id: notice_partial
    when:
      verb_id: notice_partial
    check:
      total: 1d1 + self.vigor
      success_at: 10
      partial_at: 7
    do:
      - flag:
          of: self
          id: touched
          value: true
    success:
      - flag:
          of: self
          id: clean
          value: true
    partial:
      - flag:
          of: self
          id: halfway
          value: true
    miss:
      - flag:
          of: self
          id: missed
          value: true
";

#[given(expr = "a docks table where press vigor base is {int}")]
async fn a_docks_table_where_press_vigor_base_is(world: &mut World, base: usize) {
    prepare_press(world, base as i64, true).await;
}

#[given(expr = "a docks copy where press vigor base is {int}")]
async fn a_docks_copy_where_press_vigor_base_is(world: &mut World, base: usize) {
    prepare_press(world, base as i64, false).await;
}

#[given(expr = "a docks table where Mira can partly notice")]
async fn a_docks_table_where_mira_can_partly_notice(world: &mut World) {
    let pack = copy_pack("docks", "press-notice");
    world.pack = Some(pack.clone());
    common::add_verb(&pack, NOTICE_PARTIAL_ROW);
    set_vigor(&pack, "mira", 6);
    support::stamp_pack_rev(&pack);
    listen_and_seat_both(world, &pack).await;
    world.mark = host::player_mark(world);
}

#[when(expr = "the player presses Mira")]
async fn the_player_presses_mira(world: &mut World) {
    let (mark, added) = host::player_send(world, "press mira", "\"kind\":").await;
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the GM queues Mira's partial notice")]
async fn the_gm_queues_miras_partial_notice(world: &mut World) {
    queue_mira(world, "notice_partial").await;
}

#[then(expr = "Mira is touched and clean")]
fn mira_is_touched_and_clean(world: &mut World) {
    assert_eq!(
        flags_of(world, "mira"),
        vec!["clean".to_string(), "touched".to_string()]
    );
}

#[then(expr = "Mira is touched and halfway")]
fn mira_is_touched_and_halfway(world: &mut World) {
    assert_eq!(
        flags_of(world, "mira"),
        vec!["halfway".to_string(), "touched".to_string()]
    );
}

#[then(expr = "Mira is touched and missed")]
fn mira_is_touched_and_missed(world: &mut World) {
    assert_eq!(
        flags_of(world, "mira"),
        vec!["missed".to_string(), "touched".to_string()]
    );
}

#[then(expr = "the player sees outcome {string} and no faces")]
async fn the_player_sees_outcome_and_no_faces(world: &mut World, outcome: String) {
    let needle = format!("{{\"kind\":\"check\",\"outcome\":\"{outcome}\"}}");
    let added = host::wait_player_from(world, world.mark, &needle).await;
    world.player_added = added;
    assert!(
        !world.player_added.contains("\"dice\""),
        "{}",
        world.player_added
    );
    assert!(
        !world.player_added.contains("\"success\":"),
        "{}",
        world.player_added
    );
    assert_eq!(
        world.player_added.matches("\"kind\":\"check\"").count(),
        1,
        "{}",
        world.player_added
    );
}

#[then(expr = "the GM sees outcome {string} and the faces")]
async fn the_gm_sees_outcome_and_the_faces(world: &mut World, outcome: String) {
    let text = host::wait_gm(world, "\"rolls\"").await;
    assert_outcome_dice(&read_committed(&text), &outcome);
}

#[then(expr = "the check outcome is not {string}")]
async fn the_check_outcome_is_not(world: &mut World, outcome: String) {
    let word = format!("\"outcome\":\"{outcome}\"");
    assert!(
        !world.player_added.contains(&word),
        "{}",
        world.player_added
    );
    let text = host::wait_gm(world, "\"rolls\"").await;
    assert!(!text.contains(&word), "{text}");
}

async fn prepare_press(world: &mut World, base: i64, is_seated: bool) {
    let pack = copy_pack("docks", "press");
    world.pack = Some(pack.clone());
    common::add_verb(&pack, PRESS_ROW);
    set_vigor(&pack, "rowan", base);
    super::common::edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
    });
    if is_seated {
        listen_and_seat_both(world, &pack).await;
    }
}

/// `body` holds one roll with `outcome`, totalling its first dice line. A press rolls
/// `1d1 + self.vigor` against 10.
fn assert_outcome_dice(body: &Value, outcome: &str) {
    let rolls = body["rolls"].as_array().unwrap_or_else(|| panic!("{body}"));
    assert_eq!(rolls.len(), 1, "{body}");
    let roll = &rolls[0];
    assert_eq!(roll["outcome"].as_str(), Some(outcome), "{body}");
    assert!(roll.get("success").is_none(), "{body}");
    let line = body["dice"][0].as_str().unwrap_or_else(|| panic!("{body}"));
    assert_eq!(
        roll["total"].as_i64(),
        Some(dice_line_total(line)),
        "{body}"
    );
    if line.starts_with("1d1 → ") {
        assert_eq!(roll["success_at"].as_i64(), Some(10), "{body}");
    }
}
