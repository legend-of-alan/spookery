use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, temp_pack};
use crate::World;

use super::table;

// Docks grit is 2d6 plus a meter of 1, so the total is 3..=13.
const NOTICE_ROWS: &str = "
  - id: notice_sure
    when:
      verb: notice_sure
    roll:
      check: grit
      target: 3
    success:
      - flag:
          who: self
          key: braced
          value: true
    failure:
      - flag:
          who: self
          key: slipped
          value: true
  - id: notice_hard
    when:
      verb: notice_hard
    roll:
      check: grit
      target: 14
    success:
      - flag:
          who: self
          key: braced
          value: true
    failure:
      - flag:
          who: self
          key: slipped
          value: true
";

const OPPOSE_ROWS: &str = "
  - id: mark
    when:
      verb: mark
      target: actor
    roll:
      check: grit
      target: 0
    success:
      - flag:
          who: target
          key: marked
          value: true
    failure:
      - flag:
          who: target
          key: missed
          value: true
  - id: defend
    when:
      verb: defend
    roll:
      check: grit
      target: 0
    success:
      - flag:
          who: self
          key: guarded
          value: true
";

#[when(expr = "the player rolls grit")]
fn the_player_rolls_grit(world: &mut World) {
    let (mark, added) = table::player_send(world, "grit", "\"kind\":");
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

#[then(expr = "the tick log resolves the check without faces")]
fn the_tick_log_resolves_the_check_without_faces(world: &mut World) {
    let text =
        spookery_engine::testing::journal::read_record_line(table::pack(world), "round-0001")
            .unwrap_or_else(|err| panic!("read tick: {}", err.detail));
    assert!(text.contains("\"check_rolled\""), "{text}");
    assert!(!text.contains("faces"), "{text}");
}

#[then(expr = "the GM sees the grit faces")]
fn the_gm_sees_the_grit_faces(world: &mut World) {
    let text = table::wait_gm(world, "\"kind\":\"check\"");
    let event = text
        .lines()
        .find(|line| line.contains("\"kind\":\"check\"") && line.contains("\"faces\""))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(event.contains("\"faces\":["), "{event}");
    assert!(event.contains("\"stat\":1"), "{event}");
    assert!(event.contains("\"target\":0"), "{event}");
    let result = text
        .lines()
        .find(|line| line.contains("\"checks\"") && line.contains("\"faces\""))
        .unwrap_or_else(|| panic!("tool result has no checks\n{text}"));
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
    let pack = temp_pack("npc-notice");
    world.pack = Some(pack.clone());
    append_moves(&pack, NOTICE_ROWS);
    insert_flags(&pack, "    - braced\n    - slipped\n");
    support::stamp_pack_rev(&pack);
    listen_both(world, &pack);
    world.scratch = snapshot_of(&pack).display().to_string();
    world.mark = table::player_len(world);
}

#[given(expr = "a docks table where Mira can oppose a mark")]
fn a_docks_table_where_mira_can_oppose_a_mark(world: &mut World) {
    let pack = temp_pack("npc-oppose");
    world.pack = Some(pack.clone());
    append_moves(&pack, OPPOSE_ROWS);
    insert_flags(&pack, "    - marked\n    - missed\n    - guarded\n");
    set_mira_grit(&pack, 100);
    super::common::edit_save(&pack, |now| support::place_of(now, "mira", "docks_shop"));
    listen_both(world, &pack);
    world.scratch = snapshot_of(&pack).display().to_string();
}

#[when(expr = "the GM queues Mira's sure notice")]
fn the_gm_queues_miras_sure_notice(world: &mut World) {
    queue_mira(world, "notice_sure");
}

#[when(expr = "the GM queues Mira's hard notice")]
fn the_gm_queues_miras_hard_notice(world: &mut World) {
    queue_mira(world, "notice_hard");
}

#[when(expr = "the GM queues Mira's defense")]
fn the_gm_queues_miras_defense(world: &mut World) {
    queue_mira(world, "defend");
}

#[when(expr = "the player marks Mira")]
fn the_player_marks_mira(world: &mut World) {
    let (mark, added) = table::player_send(world, "mark mira", "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "a kernel habit rolls a check for Mira")]
fn a_kernel_habit_rolls_a_check_for_mira(world: &mut World) {
    let pack = temp_pack("npc-habit");
    world.pack = Some(pack.clone());
    insert_flags(&pack, "    - braced\n    - slipped\n");
    insert_brace_habit(&pack);
    support::stamp_pack_rev(&pack);
    let before = snapshot_of(&pack);
    spookery_engine::play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    world.scratch = before.display().to_string();
}

#[when(expr = "the player names another PC")]
fn the_player_names_another_pc(world: &mut World) {
    let (mark, added) = table::player_send(world, "player:pc_nell grit", "\"kind\":\"refused\"");
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

#[then(expr = "Mira guarded and the mark missed")]
fn mira_guarded_and_the_mark_missed(world: &mut World) {
    assert_eq!(
        flags_of(world, "mira"),
        vec!["guarded".to_string(), "missed".to_string()]
    );
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
    let added = table::wait_player_from(world, world.mark, "\"kind\":\"perceive\"");
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

#[then(expr = "the GM sees both rolls")]
fn the_gm_sees_both_rolls(world: &mut World) {
    let text = table::wait_gm(world, "\"kind\":\"check\"");
    let events = check_events(&text);
    assert_eq!(events.len(), 2, "{text}");
    assert_opposed(&events);
    assert_opposed(&commit_checks(&text));
}

#[then(expr = "the tick log records Mira's kernel brace")]
fn the_tick_log_records_miras_kernel_brace(world: &mut World) {
    let tick = tick_json(world);
    let ran: Vec<_> = super::steps_run::read_steps_run(&tick)
        .into_iter()
        .filter(|step| step == "cast:mira/brace")
        .collect();
    assert_eq!(ran.len(), 1, "{tick}");
    let flags: Vec<_> = tick["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| effect["class"] == "flag" && effect["actor_id"] == "mira")
        .collect();
    assert_eq!(flags.len(), 1, "{tick}");
    assert_eq!(flags[0]["as"], "npc:mira", "{tick}");
    assert!(tick["rejected"].as_array().unwrap().is_empty(), "{tick}");
}

#[then(expr = "replaying the tick reproduces the save")]
fn replaying_the_tick_reproduces_the_save(world: &mut World) {
    let played = table::pack(world);
    let start = PathBuf::from(&world.scratch);
    let before = fs::read(played.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    let report = spookery_engine::testing::from_journal(played, Some(&start), None)
        .unwrap_or_else(|err| panic!("replay: {err}"));
    let entries = spookery_engine::testing::journal::read_entries(played)
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let expected = entries
        .iter()
        .rev()
        .find(|entry| entry.changes_state())
        .unwrap_or_else(|| panic!("no tick was written"))
        .now_hash();
    assert_eq!(
        spookery_engine::testing::journal::hash_now(&report.now_yaml),
        expected
    );
    assert_eq!(
        fs::read(played.join("state/save.yaml")).unwrap_or_else(|err| panic!("read played: {err}")),
        before
    );
    let _ = fs::remove_dir_all(&start);
}

fn listen_both(world: &mut World, pack: &Path) {
    table::listen(world, pack);
    table::seat_gm(world);
    table::seat_player(world, pack);
}

fn queue_mira(world: &mut World, move_name: &str) {
    let args = format!(r#"{{"actor_id":"mira","move":"{move_name}"}}"#);
    let text = table::gm_call(world, 4, "intent", &args);
    assert!(text.contains("\"queued\""), "{text}");
    assert!(!text.contains("UNKNOWN_KEY"), "{text}");
}

fn append_moves(pack: &Path, extra: &str) {
    let path = pack.join("rules/verbs.yaml");
    let mut moves = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read moves: {err}"));
    moves.push_str(extra);
    fs::write(&path, moves).unwrap_or_else(|err| panic!("write moves: {err}"));
}

fn insert_flags(pack: &Path, flags: &str) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read mira: {err}"));
    let next = text.replacen(
        "tick:\n  eligible: true\n",
        &format!("tick:\n  eligible: true\n  flags:\n{flags}"),
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
        "rules:\n  - id: brace\n    when: habit\n    roll:\n      check: grit\n      target: 14\n    success:\n      - flag:\n          who: self\n          key: braced\n          value: true\n    failure:\n      - flag:\n          who: self\n          key: slipped\n          value: true\n  - id: leave_door_note\n",
        1,
    );
    assert_ne!(next, text, "mira habit was not inserted");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write mira: {err}"));
}

fn set_mira_grit(pack: &Path, base: i64) {
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read now: {err}"));
    let from = "  mira:\n    kind: npc\n    place: mira_rooms\n    flags: []\n    meters:\n      grit:\n        base: 1\n";
    let to = format!("  mira:\n    kind: npc\n    place: mira_rooms\n    flags: []\n    meters:\n      grit:\n        base: {base}\n");
    let next = text.replacen(from, &to, 1);
    assert_ne!(next, text, "mira grit was not changed");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write now: {err}"));
}

fn snapshot_of(pack: &Path) -> PathBuf {
    let dest = support::fresh_dir("before");
    support::copy_dir(pack, &dest);
    dest
}

fn flags_of(world: &World, id: &str) -> Vec<String> {
    let mut flags = match table::now_of(table::pack(world)).actors.get(id) {
        Some(spookery_engine::testing::ActorLive::Npc { flags, .. })
        | Some(spookery_engine::testing::ActorLive::Pc { flags, .. }) => flags.clone(),
        _ => panic!("{id} missing"),
    };
    flags.sort();
    flags
}

fn tick_json(world: &World) -> Value {
    crate::support::read_round(table::pack(world), "round-0001")
}

fn assert_one_roll(world: &mut World, stat: i64, target: i64, outcome: &str) {
    let text = table::wait_gm(world, "\"kind\":\"check\"");
    let events = check_events(&text);
    assert_eq!(events.len(), 1, "{text}");
    assert_roll(&events[0], stat, target, outcome);
    let checks = commit_checks(&text);
    assert_eq!(checks.len(), 1, "{text}");
    assert_roll(&checks[0], stat, target, outcome);
}

fn assert_opposed(rolls: &[Value]) {
    assert_eq!(rolls.len(), 2, "{rolls:?}");
    let npc = rolls
        .iter()
        .find(|roll| roll["stat"].as_i64() == Some(100))
        .unwrap_or_else(|| panic!("{rolls:?}"));
    let pc = rolls
        .iter()
        .find(|roll| roll["stat"].as_i64() == Some(1))
        .unwrap_or_else(|| panic!("{rolls:?}"));
    let npc_total = npc["total"].as_i64().unwrap_or_else(|| panic!("{npc}"));
    assert_roll(npc, 100, 0, "success");
    assert_roll(pc, 1, npc_total, "miss");
    assert!(npc_total >= 102, "{npc}");
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

fn check_events(text: &str) -> Vec<Value> {
    let mut events = Vec::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(params) = value.get("params") else {
            continue;
        };
        if params.get("kind").and_then(Value::as_str) == Some("check") {
            events.push(params.clone());
        }
    }
    events
}

fn commit_checks(text: &str) -> Vec<Value> {
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(checks) = find_checks(&value) {
            return checks;
        }
    }
    panic!("tool result has no checks\n{text}");
}

fn find_checks(value: &Value) -> Option<Vec<Value>> {
    if let Some(checks) = value.get("checks").and_then(Value::as_array) {
        if checks.iter().any(|roll| roll.get("faces").is_some()) {
            return Some(checks.clone());
        }
    }
    match value {
        Value::Object(map) => map.values().find_map(find_checks),
        Value::Array(items) => items.iter().find_map(find_checks),
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
      target: 10
      partial_at: 7
    do:
      - flag:
          who: target
          key: touched
          value: true
    success:
      - flag:
          who: target
          key: clean
          value: true
    partial:
      - flag:
          who: target
          key: halfway
          value: true
    failure:
      - flag:
          who: target
          key: missed
          value: true
";

const NOTICE_PARTIAL: &str = "
  - id: notice_partial
    when:
      verb: notice_partial
    roll:
      check: band
      target: 10
      partial_at: 7
    do:
      - flag:
          who: self
          key: touched
          value: true
    success:
      - flag:
          who: self
          key: clean
          value: true
    partial:
      - flag:
          who: self
          key: halfway
          value: true
    failure:
      - flag:
          who: self
          key: missed
          value: true
";

const DEFEND_ROW: &str = "
  - id: defend
    when:
      verb: defend
    roll:
      check: grit
      target: 0
    success:
      - flag:
          who: self
          key: guarded
          value: true
";

const BAND_CHECK: &str = "
  - id: band
    dice:
      count: 1
      sides: 1
    stat: grit
";

const PRESS_FLAGS: &str =
    "    - touched\n    - clean\n    - halfway\n    - missed\n    - guarded\n";

#[given(expr = "a docks table where press grit base is {int}")]
fn a_docks_table_where_press_grit_base_is(world: &mut World, base: usize) {
    prepare_press(world, base as i64, true);
}

#[given(expr = "a docks copy where press grit base is {int}")]
fn a_docks_copy_where_press_grit_base_is(world: &mut World, base: usize) {
    prepare_press(world, base as i64, false);
}

#[given(expr = "a docks table where Mira opposes a three-band press")]
fn a_docks_table_where_mira_opposes_a_three_band_press(world: &mut World) {
    let pack = temp_pack("press-oppose");
    world.pack = Some(pack.clone());
    append_moves(&pack, PRESS_ROW);
    append_moves(&pack, DEFEND_ROW);
    append_check(&pack, BAND_CHECK);
    insert_flags(&pack, PRESS_FLAGS);
    set_rowan_grit(&pack, 6);
    set_mira_grit(&pack, 100);
    super::common::edit_save(&pack, |now| support::place_of(now, "mira", "docks_shop"));
    listen_both(world, &pack);
    world.scratch = snapshot_of(&pack).display().to_string();
}

#[given(expr = "a docks table where Mira can partly notice")]
fn a_docks_table_where_mira_can_partly_notice(world: &mut World) {
    let pack = temp_pack("press-notice");
    world.pack = Some(pack.clone());
    append_moves(&pack, NOTICE_PARTIAL);
    append_check(&pack, BAND_CHECK);
    insert_flags(&pack, PRESS_FLAGS);
    set_mira_grit(&pack, 6);
    support::stamp_pack_rev(&pack);
    listen_both(world, &pack);
    world.scratch = snapshot_of(&pack).display().to_string();
    world.mark = table::player_len(world);
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

#[when(expr = "the moves file gains {string}")]
fn the_moves_file_gains(world: &mut World, fault: String) {
    let row = match fault.as_str() {
        "partial_at without partial" => {
            "\n  - id: press\n    when:\n      verb: press\n    roll:\n      check: grit\n      target: 10\n      partial_at: 7\n    do:\n      - flag:\n          who: self\n          key: touched\n          value: true\n"
        }
        "partial without partial_at" => {
            "\n  - id: press\n    when:\n      verb: press\n    roll:\n      check: grit\n      target: 10\n    partial:\n      - flag:\n          who: self\n          key: halfway\n          value: true\n"
        }
        "partial_at above difficulty" => {
            "\n  - id: press\n    when:\n      verb: press\n    roll:\n      check: grit\n      target: 10\n      partial_at: 10\n    success:\n      - flag:\n          who: self\n          key: touched\n          value: true\n    partial:\n      - flag:\n          who: self\n          key: halfway\n          value: true\n"
        }
        "partial_at without a check" => {
            "\n  - id: press\n    when:\n      verb: press\n    partial:\n      - flag:\n          who: self\n          key: halfway\n          value: true\n"
        }
        other => panic!("unknown partial fault {other}"),
    };
    let pack = table::pack(world).to_path_buf();
    append_moves(&pack, row);
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

#[then(expr = "Mira guarded and the press missed")]
fn mira_guarded_and_the_press_missed(world: &mut World) {
    assert_eq!(
        flags_of(world, "mira"),
        vec![
            "guarded".to_string(),
            "missed".to_string(),
            "touched".to_string()
        ]
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
    let text = table::wait_gm(world, "\"kind\":\"check\"");
    let events = check_events(&text);
    assert_eq!(events.len(), 1, "{text}");
    assert_outcome_faces(&events[0], &outcome);
    let checks = commit_checks(&text);
    assert_eq!(checks.len(), 1, "{text}");
    assert_outcome_faces(&checks[0], &outcome);
}

#[then(expr = "the check outcome is not {string}")]
fn the_check_outcome_is_not(world: &mut World, outcome: String) {
    let word = format!("\"outcome\":\"{outcome}\"");
    assert!(
        !world.player_added.contains(&word),
        "{}",
        world.player_added
    );
    let text = table::wait_gm(world, "\"kind\":\"check\"");
    assert!(!text.contains(&word), "{text}");
}

#[then(expr = "the GM sees a binary opposed press")]
fn the_gm_sees_a_binary_opposed_press(world: &mut World) {
    let text = table::wait_gm(world, "\"kind\":\"check\"");
    assert_binary_opposed(&check_events(&text));
    assert_binary_opposed(&commit_checks(&text));
}

fn prepare_press(world: &mut World, base: i64, sit: bool) {
    let pack = temp_pack("press");
    world.pack = Some(pack.clone());
    append_moves(&pack, PRESS_ROW);
    append_check(&pack, BAND_CHECK);
    insert_flags(&pack, PRESS_FLAGS);
    set_rowan_grit(&pack, base);
    super::common::edit_save(&pack, |now| support::place_of(now, "mira", "docks_shop"));
    if sit {
        listen_both(world, &pack);
        world.scratch = snapshot_of(&pack).display().to_string();
    }
}

fn append_check(pack: &Path, extra: &str) {
    let path = pack.join("rules/checks.yaml");
    let mut text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read checks: {err}"));
    text.push_str(extra);
    fs::write(&path, text).unwrap_or_else(|err| panic!("write checks: {err}"));
}

fn set_rowan_grit(pack: &Path, base: i64) {
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read now: {err}"));
    let from = "  pc_rowan:\n    kind: pc\n    place: docks_shop\n    flags: []\n    meters:\n      grit:\n        base: 1\n";
    let to = format!("  pc_rowan:\n    kind: pc\n    place: docks_shop\n    flags: []\n    meters:\n      grit:\n        base: {base}\n");
    let next = text.replacen(from, &to, 1);
    assert_ne!(next, text, "rowan grit was not changed");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write now: {err}"));
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

fn assert_binary_opposed(rolls: &[Value]) {
    assert_eq!(rolls.len(), 2, "{rolls:?}");
    let npc = rolls
        .iter()
        .find(|roll| roll["stat"].as_i64() == Some(100))
        .unwrap_or_else(|| panic!("{rolls:?}"));
    let pc = rolls
        .iter()
        .find(|roll| roll["stat"].as_i64() == Some(6))
        .unwrap_or_else(|| panic!("{rolls:?}"));
    let npc_total = npc["total"].as_i64().unwrap_or_else(|| panic!("{npc}"));
    assert_eq!(npc["outcome"].as_str(), Some("success"), "{npc}");
    assert_eq!(npc["target"].as_i64(), Some(0), "{npc}");
    assert!(
        npc["faces"]
            .as_array()
            .is_some_and(|faces| !faces.is_empty()),
        "{npc}"
    );
    assert_eq!(pc["outcome"].as_str(), Some("miss"), "{pc}");
    assert_eq!(pc["target"].as_i64(), Some(npc_total), "{pc}");
    assert!(npc_total >= 102, "{npc}");
    assert!(
        rolls.iter().all(|roll| roll["outcome"] != "partial"),
        "{rolls:?}"
    );
    assert!(
        rolls.iter().all(|roll| roll.get("success").is_none()),
        "{rolls:?}"
    );
}
