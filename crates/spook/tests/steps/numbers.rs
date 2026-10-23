//! Dice and arithmetic in rule numbers.

use std::fs;

use cucumber::{then, when};
use serde_json::Value;
use spookery_engine::journal::list_tick_keys;

use super::table;
use crate::World;

#[when(expr = "the pack's hit adds {string} to grit")]
fn the_packs_hit_adds(world: &mut World, add: String) {
    append_hit(world, "self", &format!("\"{add}\""));
}

#[when(expr = "the pack's hit raises the grit of {string}")]
fn the_packs_hit_raises_the_grit_of(world: &mut World, who: String) {
    append_hit(world, &who, "1");
}

#[when(expr = "Mira's habit adds {string} to grit")]
fn miras_habit_adds(world: &mut World, add: String) {
    let pack = table::pack(world).to_path_buf();
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read mira: {err}"));
    let next = text.replacen(
        "rules:\n  - id: leave_door_note\n",
        &format!(
            "rules:\n  - id: toss\n    when: habit\n    do:\n      - meter:\n          who: self\n          key: grit\n          add: \"{add}\"\n  - id: leave_door_note\n"
        ),
        1,
    );
    assert_ne!(next, text, "mira habit was not inserted");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write mira: {err}"));
    crate::support::stamp_pack_rev(&pack);
}

#[when(expr = "one round plays")]
fn one_round_plays(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    spookery_engine::play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
}

#[when(expr = "the pack's hit requires grit of at least {string}")]
fn the_packs_hit_requires(world: &mut World, value: String) {
    let pack = table::pack(world).to_path_buf();
    let path = pack.join("rules/verbs.yaml");
    let mut text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read verbs: {err}"));
    text.push_str(&format!(
        "\n  - id: hit\n    when:\n      verb: hit\n    if:\n      - meter:\n          who: self\n          key: grit\n          op: \">=\"\n          value: \"{value}\"\n    do:\n      - meter:\n          who: self\n          key: grit\n          add: 1\n"
    ));
    fs::write(&path, text).unwrap_or_else(|err| panic!("write verbs: {err}"));
}

#[when(expr = "the player hits and the GM commits")]
fn the_player_hits_and_the_gm_commits(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let snap = crate::support::fresh_dir("before-hit");
    crate::support::copy_dir(&pack, &snap);
    world.scratch = snap.display().to_string();
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    table::seat_gm(world);
    let _ = table::player_send(world, "hit", "\"kind\":\"queued\"");
    world.mark = table::player_len(world);
    world.stdout = table::gm_call(world, 7, "commit_round", r#"{"include_tick":true}"#);
}

#[then(expr = "Rowan's grit is {int}")]
fn rowans_grit_is(world: &mut World, expected: i64) {
    assert_eq!(grit(world), expected);
}

#[then(expr = "Mira's grit rose by the 1d4 face")]
fn miras_grit_rose_by_the_1d4_face(world: &mut World) {
    let faces = dice_faces(world, "1d4");
    assert_eq!(faces.len(), 1, "{faces:?}");
    assert_eq!(grit_of(world, "mira"), 1 + i64::from(faces[0]));
}

#[then(expr = "the journal records the 1d4 face")]
fn the_journal_records_the_1d4_face(world: &mut World) {
    let line = dice_line(world);
    assert!(line.contains("1d4 →"), "{line}");
}

#[then(expr = "the report names the line holding {string}")]
fn the_report_names_the_line_holding(world: &mut World, needle: String) {
    let pack = table::pack(world);
    let text = fs::read_to_string(pack.join("rules/verbs.yaml"))
        .unwrap_or_else(|err| panic!("read verbs: {err}"));
    let mut line = None;
    for (index, item) in text.lines().enumerate() {
        if item.contains(&needle) {
            line = Some(index + 1);
        }
    }
    let line = line.unwrap_or_else(|| panic!("no line holding {needle}"));
    let located = format!("rules/verbs.yaml:{line}:");
    assert!(
        world.stderr.contains(&located),
        "missing {located}\n{}",
        world.stderr
    );
}

#[then(expr = "Rowan's grit rose by the 2d6 faces")]
fn rowans_grit_rose_by_the_2d6_faces(world: &mut World) {
    let faces = dice_faces(world, "2d6");
    let sum: i64 = faces.iter().map(|face| i64::from(*face)).sum();
    assert_eq!(faces.len(), 2, "{faces:?}");
    assert_eq!(grit(world), 1 + sum);
}

#[then(expr = "Rowan's grit rose by the higher d20")]
fn rowans_grit_rose_by_the_higher_d20(world: &mut World) {
    let faces = dice_faces(world, "2d20kh1");
    let high = faces.iter().copied().max().unwrap_or(0);
    assert_eq!(grit(world), 1 + i64::from(high));
}

#[then(expr = "the journal records the 2d6 faces")]
fn the_journal_records_the_2d6_faces(world: &mut World) {
    let line = dice_line(world);
    assert!(line.contains("2d6 →"), "{line}");
    assert!(line.contains(" = "), "{line}");
}

#[then(expr = "the journal records both d20 faces")]
fn the_journal_records_both_d20_faces(world: &mut World) {
    let faces = dice_faces(world, "2d20kh1");
    assert_eq!(faces.len(), 2, "{faces:?}");
}

#[then(expr = "the GM commit reply shows the 2d6 faces")]
fn the_gm_commit_reply_shows_the_2d6_faces(world: &mut World) {
    assert!(
        world.stdout.contains("2d6 →"),
        "missing dice\n{}",
        world.stdout
    );
}

#[then(expr = "the player text has no dice faces")]
fn the_player_text_has_no_dice_faces(world: &mut World) {
    let added = table::wait_player_from(world, world.mark, "\"kind\":");
    assert!(!added.contains('→'), "{added}");
    assert!(!added.contains("2d6"), "{added}");
}

#[then(expr = "the report names the expression line")]
fn the_report_names_the_expression_line(world: &mut World) {
    let pack = table::pack(world);
    let text = fs::read_to_string(pack.join("rules/verbs.yaml"))
        .unwrap_or_else(|err| panic!("read verbs: {err}"));
    let mut line = None;
    for (index, item) in text.lines().enumerate() {
        if item.contains("add:") {
            line = Some(index + 1);
        }
    }
    let line = line.unwrap_or_else(|| panic!("no add line"));
    let located = format!("rules/verbs.yaml:{line}:");
    assert!(
        world.stderr.contains(&located),
        "missing {located}\n{}",
        world.stderr
    );
}

#[then(expr = "the journal rejects the hit as OUT_OF_RANGE")]
fn the_journal_rejects_the_hit_as_out_of_range(world: &mut World) {
    let tick = latest_tick(world);
    let rejected = tick["rejected"]
        .as_array()
        .unwrap_or_else(|| panic!("rejected\n{tick}"));
    assert!(
        rejected.iter().any(|row| row["reason"] == "OUT_OF_RANGE"),
        "{tick}"
    );
}

fn append_hit(world: &World, who: &str, add: &str) {
    let pack = table::pack(world).to_path_buf();
    let path = pack.join("rules/verbs.yaml");
    let mut text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read verbs: {err}"));
    text.push_str(&format!(
        "\n  - id: hit\n    when:\n      verb: hit\n    do:\n      - meter:\n          who: {who}\n          key: grit\n          add: {add}\n"
    ));
    fs::write(&path, &text).unwrap_or_else(|err| panic!("write verbs: {err}"));
    crate::support::stamp_pack_rev(&pack);
}

fn grit(world: &World) -> i64 {
    grit_of(world, "pc_rowan")
}

fn grit_of(world: &World, actor: &str) -> i64 {
    let now = table::now_of(table::pack(world));
    let live = now
        .actors
        .get(actor)
        .unwrap_or_else(|| panic!("{actor} missing"))
        .meters()
        .get("grit")
        .unwrap_or_else(|| panic!("grit missing"));
    live.base + live.offset
}

fn latest_tick(world: &World) -> Value {
    let pack = table::pack(world);
    let keys = list_tick_keys(pack);
    let name = keys.last().unwrap_or_else(|| panic!("no tick"));
    let text = spookery_engine::journal::read_record_line(pack, name)
        .unwrap_or_else(|err| panic!("read {name}: {}", err.detail));
    serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("{name}: {err}"))
}

fn dice_line(world: &World) -> String {
    let tick = latest_tick(world);
    tick["dice"]
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no dice line\n{tick}"))
        .to_string()
}

fn dice_faces(world: &World, word: &str) -> Vec<u32> {
    let line = dice_line(world);
    assert!(line.contains(word), "{line}");
    let after = line
        .split('→')
        .nth(1)
        .unwrap_or_else(|| panic!("no faces\n{line}"));
    let listed = after.split('=').next().unwrap_or(after);
    listed
        .split(',')
        .map(|face| {
            face.trim()
                .parse()
                .unwrap_or_else(|err| panic!("face {face}: {err}"))
        })
        .collect()
}
