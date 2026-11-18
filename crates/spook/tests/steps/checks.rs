use spookery_engine::component;
use spookery_engine::testing::world_of;
use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, copy_pack};
use crate::World;

use super::{common, table};

// Docks exert is 2d6 plus a vigor of 1, so the total is 3..=13.
const NOTICE_ROWS: &str = "
  - id: notice_sure
    when:
      verb: notice_sure
    roll:
      check: exert
      threshold: 3
    success:
      - flag:
          of: self
          key: braced
          value: true
    miss:
      - flag:
          of: self
          key: slipped
          value: true
  - id: notice_hard
    when:
      verb: notice_hard
    roll:
      check: exert
      threshold: 14
    success:
      - flag:
          of: self
          key: braced
          value: true
    miss:
      - flag:
          of: self
          key: slipped
          value: true
";

// The mark's threshold rolls its own dice and reads the target's vigor: a defense written in
// the pack.
const CONTESTED_MARK_ROW: &str = "
  - id: mark
    when:
      verb: mark
      target: actor
    roll:
      check: exert
      threshold: \"2d6 + target.vigor\"
    success:
      - flag:
          of: target
          key: marked
          value: true
    miss:
      - flag:
          of: target
          key: missed
          value: true
";

#[when(expr = "the player rolls exert")]
fn the_player_rolls_exert(world: &mut World) {
    let (mark, added) = table::player_send(world, "exert", "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the player sees a check success")]
fn the_player_sees_a_check_success(world: &mut World) {
    world.player_added = table::wait_player_from(
        world,
        world.mark,
        "{\"kind\":\"check\",\"outcome\":\"success\"}",
    );
    assert!(
        !world.player_added.contains("\"faces\""),
        "{}",
        world.player_added
    );
    assert!(
        !world.player_added.contains("\"success\":"),
        "{}",
        world.player_added
    );
}

#[then(expr = "the journal records the check without faces")]
fn the_journal_records_the_check_without_faces(world: &mut World) {
    let text =
        spookery_engine::testing::journal::read_record_line(table::pack(world), "round-0001")
            .unwrap_or_else(|err| panic!("read round: {}", err.detail));
    assert!(!text.contains("faces"), "{text}");
}

#[then(expr = "the GM sees the exert faces")]
fn the_gm_sees_the_exert_faces(world: &mut World) {
    let text = table::wait_gm(world, "\"rolls\"");
    let roll = read_committed_rolls(&text)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("{text}"));
    assert!(roll["faces"].is_array(), "{roll}");
    assert_eq!(roll["stat"], 1, "{roll}");
    assert_eq!(roll["target"], 0, "{roll}");
    let result = text
        .lines()
        .find(|line| line.contains("\"rolls\"") && line.contains("\"faces\""))
        .unwrap_or_else(|| panic!("tool result has no rolls\n{text}"));
    assert!(result.contains("\"faces\":["), "{result}");
    assert!(result.contains("\"stat\":"), "{result}");
    assert!(result.contains("\"outcome\":"), "{result}");
    assert!(!result.contains("\"success\":"), "{result}");
}

#[then(expr = "the player sees a check result without faces")]
fn the_player_sees_a_check_result_without_faces(world: &mut World) {
    let added = table::wait_player_from(world, world.mark, "{\"kind\":\"check\",\"outcome\":");
    assert!(
        added.contains("{\"kind\":\"check\",\"outcome\":\"success\"}")
            || added.contains("{\"kind\":\"check\",\"outcome\":\"partial\"}")
            || added.contains("{\"kind\":\"check\",\"outcome\":\"miss\"}"),
        "{added}"
    );
    assert!(!added.contains("\"success\":"), "{added}");
    assert!(!added.contains("\"faces\""), "{added}");
}

#[given(expr = "a docks table where Mira can notice")]
fn a_docks_table_where_mira_can_notice(world: &mut World) {
    let pack = copy_pack("docks", "npc-notice");
    world.pack = Some(pack.clone());
    append_verbs(&pack, NOTICE_ROWS);
    insert_mira_flags(&pack, "    - braced\n    - slipped\n");
    support::stamp_pack_rev(&pack);
    listen_and_seat_both(world, &pack);
    world.scratch = snapshot_pack(&pack).display().to_string();
    world.mark = table::player_mark(world);
}

#[given(expr = "a docks table where Mira's vigor contests a mark")]
fn a_docks_table_where_miras_vigor_contests_a_mark(world: &mut World) {
    let pack = copy_pack("docks", "contested-mark");
    world.pack = Some(pack.clone());
    append_verbs(&pack, CONTESTED_MARK_ROW);
    insert_mira_flags(&pack, "    - marked\n    - missed\n");
    set_vigor(&pack, "mira", 100);
    super::common::edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
    });
    listen_and_seat_both(world, &pack);
    world.scratch = snapshot_pack(&pack).display().to_string();
}

#[when(expr = "the GM queues Mira's sure notice")]
fn the_gm_queues_miras_sure_notice(world: &mut World) {
    queue_mira(world, "notice_sure");
}

#[when(expr = "the GM queues Mira's hard notice")]
fn the_gm_queues_miras_hard_notice(world: &mut World) {
    queue_mira(world, "notice_hard");
}

#[when(expr = "an engine habit rolls a check for Mira")]
fn an_engine_habit_rolls_a_check_for_mira(world: &mut World) {
    let pack = copy_pack("docks", "npc-habit");
    world.pack = Some(pack.clone());
    insert_mira_flags(&pack, "    - braced\n    - slipped\n");
    insert_brace_habit(&pack);
    support::stamp_pack_rev(&pack);
    let before = snapshot_pack(&pack);
    spookery_engine::play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    world.scratch = before.display().to_string();
}

#[when(expr = "the player sends a line with a player prefix")]
fn the_player_sends_a_line_with_a_player_prefix(world: &mut World) {
    let (mark, added) = table::player_send(world, "player:nell exert", "\"kind\":\"refused\"");
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
fn the_gm_sees_miras_sure_notice(world: &mut World) {
    assert_one_roll(world, 1, 3, "success");
}

#[then(expr = "the GM sees Mira's failed notice")]
fn the_gm_sees_miras_failed_notice(world: &mut World) {
    assert_one_roll(world, 1, 14, "miss");
}

#[then(expr = "the player sees no check frame")]
fn the_player_sees_no_check_frame(world: &mut World) {
    let added = table::wait_player_from(world, world.mark, "\"kind\":\"view\"");
    assert!(!added.contains("\"kind\":\"check\""), "{added}");
}

#[then(expr = "the player sees only their own check")]
fn the_player_sees_only_their_own_check(world: &mut World) {
    let added = table::wait_player_from(
        world,
        world.mark,
        "{\"kind\":\"check\",\"outcome\":\"miss\"}",
    );
    assert_eq!(added.matches("\"kind\":\"check\"").count(), 1, "{added}");
    assert!(!added.contains("\"faces\""), "{added}");
    assert!(!added.contains("\"success\":"), "{added}");
}

#[then(expr = "the GM sees one roll and the threshold's dice line")]
fn the_gm_sees_one_roll_and_the_thresholds_dice_line(world: &mut World) {
    let body = table::read_reply(&world.stdout);
    let rolls = body["rolls"].as_array().unwrap_or_else(|| panic!("{body}"));
    assert_eq!(rolls.len(), 1, "{body}");
    // 2d6 plus Mira's vigor of 100.
    let target = rolls[0]["target"]
        .as_i64()
        .unwrap_or_else(|| panic!("{body}"));
    assert!((102..=112).contains(&target), "{body}");
    assert_roll(&rolls[0], 1, target, "miss");
    let dice = body["dice"].as_array().unwrap_or_else(|| panic!("{body}"));
    assert_eq!(dice.len(), 1, "{body}");
    assert!(
        dice[0].as_str().is_some_and(|line| line.contains("2d6 →")),
        "{body}"
    );
}

#[then(expr = "the journal records Mira's engine brace")]
fn the_journal_records_miras_engine_brace(world: &mut World) {
    let round = common::read_first_round(world);
    let ran: Vec<_> = super::record::read_extra_steps(&round)
        .into_iter()
        .filter(|step| step == "cast:mira/brace")
        .collect();
    assert_eq!(ran.len(), 1, "{round}");
    let flags: Vec<_> = round["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| effect["kind"] == "flag" && effect["entity"]["actor"] == "mira")
        .collect();
    assert_eq!(flags.len(), 1, "{round}");
    assert_eq!(flags[0]["as"], "npc:mira", "{round}");
    assert!(round["refused"].as_array().unwrap().is_empty(), "{round}");
}

#[when(expr = "the save before the fight is kept")]
fn the_save_before_the_fight_is_kept(world: &mut World) {
    world.scratch = snapshot_pack(table::pack(world)).display().to_string();
}

#[then(expr = "play shows {int} successful rolls")]
fn play_shows_successful_rolls(world: &mut World, rolls: usize) {
    let succeeded = |stdout: &str| stdout.matches("Your roll succeeded.").count() >= rolls;
    common::wait_play_until(world, succeeded, &format!("{rolls} successful rolls"));
}

#[then(expr = "briar's hp is {int}")]
fn briars_hp_is(world: &mut World, expected: i64) {
    let pack = table::pack(world).to_path_buf();
    let briar = spookery_engine::entity::Id::Actor("briar".into());
    table::poll_until(&format!("briar hp {expected}"), || {
        let now = world_of(&pack);
        match now.entities().get::<component::Meter>(&briar).value("hp") {
            Ok(hp) if hp == expected => Ok(()),
            held => Err(format!("hp {held:?}")),
        }
    });
}

#[then(expr = "replaying the round reproduces the save")]
fn replaying_the_round_reproduces_the_save(world: &mut World) {
    let played = table::pack(world);
    let start = PathBuf::from(&world.scratch);
    let before = fs::read(played.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    let entries = spookery_engine::testing::journal::read_records(played)
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let report =
        spookery_engine::testing::replay::apply_journal(played, &entries, Some(&start), None)
            .unwrap_or_else(|err| panic!("replay: {err}"));
    let last = entries
        .iter()
        .rev()
        .find(|entry| entry.changes_state())
        .unwrap_or_else(|| panic!("no round was written"));
    assert_eq!(
        spookery_engine::testing::journal::blake2s_tagged(&report.now_yaml),
        spookery_engine::testing::journal::state_hash(last)
    );
    assert_eq!(
        fs::read(played.join("state/save.yaml")).unwrap_or_else(|err| panic!("read played: {err}")),
        before
    );
    let _ = fs::remove_dir_all(&start);
}

fn listen_and_seat_both(world: &mut World, pack: &Path) {
    table::listen(world, pack);
    table::seat_gm(world);
    table::seat_rowan(world, pack);
}

fn queue_mira(world: &mut World, move_id: &str) {
    let args = format!(r#"{{"actor_id":"mira","move":"{move_id}"}}"#);
    let text = table::gm_tool_call(world, 4, "queue", &args);
    assert!(text.contains("\"id\""), "{text}");
    assert!(!text.contains("UNKNOWN_ID"), "{text}");
}

fn append_verbs(pack: &Path, extra: &str) {
    let path = pack.join("rules/verbs.yaml");
    let mut moves = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read moves: {err}"));
    moves.push_str(extra);
    fs::write(&path, moves).unwrap_or_else(|err| panic!("write moves: {err}"));
}

fn insert_mira_flags(pack: &Path, flags: &str) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read mira: {err}"));
    let next = text.replacen(
        "idle:\n  eligible: true\n",
        &format!("idle:\n  eligible: true\nflags:\n{flags}"),
        1,
    );
    assert_ne!(next, text, "mira flags were not inserted");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write mira: {err}"));
}

fn insert_brace_habit(pack: &Path) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read mira: {err}"));
    let next = text.replacen(
        "rules:\n  - id: leave_door_note\n",
        "rules:\n  - id: brace\n    when: habit\n    roll:\n      check: exert\n      threshold: 14\n    success:\n      - flag:\n          of: self\n          key: braced\n          value: true\n    miss:\n      - flag:\n          of: self\n          key: slipped\n          value: true\n  - id: leave_door_note\n",
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

fn snapshot_pack(pack: &Path) -> PathBuf {
    let dest = support::fresh_dir("before");
    support::copy_dir(pack, &dest);
    dest
}

fn flags_of(world: &World, id: &str) -> Vec<String> {
    let now = world_of(table::pack(world));
    assert!(now.actors().contains(&id.into()), "{id} missing");
    let id = id.into();
    let flags = now.actors().get::<component::Flag>(&id);
    flags.iter().map(|(flag, ())| flag.to_string()).collect()
}

fn assert_one_roll(world: &mut World, stat: i64, target: i64, outcome: &str) {
    let text = table::wait_gm(world, "\"rolls\"");
    let rolls = read_committed_rolls(&text);
    assert_eq!(rolls.len(), 1, "{text}");
    assert_roll(&rolls[0], stat, target, outcome);
}

fn assert_roll(roll: &Value, stat: i64, target: i64, outcome: &str) {
    let faces = roll["faces"].as_array().unwrap_or_else(|| panic!("{roll}"));
    assert_eq!(faces.len(), 2, "{roll}");
    let face_sum: i64 = faces
        .iter()
        .map(|face| face.as_i64().unwrap_or_else(|| panic!("{roll}")))
        .sum();
    assert_eq!(roll["stat"].as_i64(), Some(stat), "{roll}");
    assert_eq!(roll["total"].as_i64(), Some(face_sum + stat), "{roll}");
    assert_eq!(roll["target"].as_i64(), Some(target), "{roll}");
    assert_eq!(roll["outcome"].as_str(), Some(outcome), "{roll}");
    assert!(roll.get("success").is_none(), "{roll}");
}

fn read_committed_rolls(text: &str) -> Vec<Value> {
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(rolls) = find_rolls(&value) {
            return rolls;
        }
    }
    panic!("tool result has no rolls\n{text}");
}

fn find_rolls(value: &Value) -> Option<Vec<Value>> {
    if let Some(rolls) = value.get("rolls").and_then(Value::as_array) {
        if rolls.iter().any(|roll| roll.get("faces").is_some()) {
            return Some(rolls.clone());
        }
    }
    match value {
        Value::Object(map) => map.values().find_map(find_rolls),
        Value::Array(items) => items.iter().find_map(find_rolls),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

const PRESS_ROW: &str = "
  - id: press
    when:
      verb: press
      target: actor
    roll:
      check: band
      threshold: 10
      partial_at: 7
    do:
      - flag:
          of: target
          key: touched
          value: true
    success:
      - flag:
          of: target
          key: clean
          value: true
    partial:
      - flag:
          of: target
          key: halfway
          value: true
    miss:
      - flag:
          of: target
          key: missed
          value: true
";

const NOTICE_PARTIAL_ROW: &str = "
  - id: notice_partial
    when:
      verb: notice_partial
    roll:
      check: band
      threshold: 10
      partial_at: 7
    do:
      - flag:
          of: self
          key: touched
          value: true
    success:
      - flag:
          of: self
          key: clean
          value: true
    partial:
      - flag:
          of: self
          key: halfway
          value: true
    miss:
      - flag:
          of: self
          key: missed
          value: true
";

const BAND_CHECK: &str = "
  - id: band
    dice:
      count: 1
      sides: 1
    meter: vigor
";

const PRESS_FLAGS: &str = "    - touched\n    - clean\n    - halfway\n    - missed\n";

#[given(expr = "a docks table where press vigor base is {int}")]
fn a_docks_table_where_press_vigor_base_is(world: &mut World, base: usize) {
    prepare_press(world, base as i64, true);
}

#[given(expr = "a docks copy where press vigor base is {int}")]
fn a_docks_copy_where_press_vigor_base_is(world: &mut World, base: usize) {
    prepare_press(world, base as i64, false);
}

#[given(expr = "a docks table where Mira can partly notice")]
fn a_docks_table_where_mira_can_partly_notice(world: &mut World) {
    let pack = copy_pack("docks", "press-notice");
    world.pack = Some(pack.clone());
    append_verbs(&pack, NOTICE_PARTIAL_ROW);
    append_check(&pack, BAND_CHECK);
    insert_mira_flags(&pack, PRESS_FLAGS);
    set_vigor(&pack, "mira", 6);
    support::stamp_pack_rev(&pack);
    listen_and_seat_both(world, &pack);
    world.scratch = snapshot_pack(&pack).display().to_string();
    world.mark = table::player_mark(world);
}

#[when(expr = "the player presses Mira")]
fn the_player_presses_mira(world: &mut World) {
    let (mark, added) = table::player_send(world, "press mira", "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the GM queues Mira's partial notice")]
fn the_gm_queues_miras_partial_notice(world: &mut World) {
    queue_mira(world, "notice_partial");
}

#[when(expr = "the verbs file gains {string}")]
fn the_verbs_file_gains(world: &mut World, fault: String) {
    let row = match fault.as_str() {
        "partial_at without partial" => {
            "\n  - id: press\n    when:\n      verb: press\n    roll:\n      check: exert\n      threshold: 10\n      partial_at: 7\n    do:\n      - flag:\n          of: self\n          key: touched\n          value: true\n"
        }
        "partial without partial_at" => {
            "\n  - id: press\n    when:\n      verb: press\n    roll:\n      check: exert\n      threshold: 10\n    partial:\n      - flag:\n          of: self\n          key: halfway\n          value: true\n"
        }
        "partial_at above difficulty" => {
            "\n  - id: press\n    when:\n      verb: press\n    roll:\n      check: exert\n      threshold: 10\n      partial_at: 10\n    success:\n      - flag:\n          of: self\n          key: touched\n          value: true\n    partial:\n      - flag:\n          of: self\n          key: halfway\n          value: true\n"
        }
        "partial_at without a check" => {
            "\n  - id: press\n    when:\n      verb: press\n    partial:\n      - flag:\n          of: self\n          key: halfway\n          value: true\n"
        }
        other => panic!("unknown partial fault {other}"),
    };
    let pack = table::pack(world).to_path_buf();
    append_verbs(&pack, row);
    support::stamp_pack_rev(&pack);
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
fn the_player_sees_outcome_and_no_faces(world: &mut World, outcome: String) {
    let needle = format!("{{\"kind\":\"check\",\"outcome\":\"{outcome}\"}}");
    let added = table::wait_player_from(world, world.mark, &needle);
    world.player_added = added;
    assert!(
        !world.player_added.contains("\"faces\""),
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
fn the_gm_sees_outcome_and_the_faces(world: &mut World, outcome: String) {
    let text = table::wait_gm(world, "\"rolls\"");
    let rolls = read_committed_rolls(&text);
    assert_eq!(rolls.len(), 1, "{text}");
    assert_outcome_faces(&rolls[0], &outcome);
}

#[then(expr = "the check outcome is not {string}")]
fn the_check_outcome_is_not(world: &mut World, outcome: String) {
    let word = format!("\"outcome\":\"{outcome}\"");
    assert!(
        !world.player_added.contains(&word),
        "{}",
        world.player_added
    );
    let text = table::wait_gm(world, "\"rolls\"");
    assert!(!text.contains(&word), "{text}");
}

fn prepare_press(world: &mut World, base: i64, sit: bool) {
    let pack = copy_pack("docks", "press");
    world.pack = Some(pack.clone());
    append_verbs(&pack, PRESS_ROW);
    append_check(&pack, BAND_CHECK);
    insert_mira_flags(&pack, PRESS_FLAGS);
    set_vigor(&pack, "rowan", base);
    super::common::edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
    });
    if sit {
        listen_and_seat_both(world, &pack);
        world.scratch = snapshot_pack(&pack).display().to_string();
    }
}

fn append_check(pack: &Path, extra: &str) {
    let path = pack.join("rules/checks.yaml");
    let mut text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read checks: {err}"));
    text.push_str(extra);
    fs::write(&path, text).unwrap_or_else(|err| panic!("write checks: {err}"));
}

fn assert_outcome_faces(roll: &Value, outcome: &str) {
    let faces = roll["faces"].as_array().unwrap_or_else(|| panic!("{roll}"));
    assert!(!faces.is_empty(), "{roll}");
    assert_eq!(roll["outcome"].as_str(), Some(outcome), "{roll}");
    assert!(roll.get("stat").and_then(Value::as_i64).is_some(), "{roll}");
    assert!(
        roll.get("total").and_then(Value::as_i64).is_some(),
        "{roll}"
    );
    assert!(
        roll.get("target").and_then(Value::as_i64).is_some(),
        "{roll}"
    );
    assert!(roll.get("success").is_none(), "{roll}");
    if faces.len() == 1 {
        assert_eq!(faces[0].as_i64(), Some(1), "{roll}");
        let stat = roll["stat"].as_i64().unwrap_or_else(|| panic!("{roll}"));
        assert_eq!(roll["total"].as_i64(), Some(stat + 1), "{roll}");
        assert_eq!(roll["target"].as_i64(), Some(10), "{roll}");
    }
}
