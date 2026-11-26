//! Dice and arithmetic in rule numbers.

use spookery_engine::component;
use spookery_engine::testing::world_of;
use std::fs;

use cucumber::{given, then, when};

use super::host;
use crate::World;

#[when(expr = "the pack's hit adds {string} to vigor")]
fn the_packs_hit_adds(world: &mut World, add: String) {
    append_hit(world, "self", &format!("\"{add}\""));
}

#[when(expr = "the pack's hit raises the vigor of {string}")]
fn the_packs_hit_raises_the_vigor_of(world: &mut World, of: String) {
    append_hit(world, &of, "1");
}

#[when(expr = "Mira's habit adds {string} to vigor")]
fn miras_habit_adds(world: &mut World, add: String) {
    insert_miras_habit(world, "", &add);
}

#[when(expr = "Mira's habit stores {string} and adds {string} to vigor")]
fn miras_habit_stores_and_adds(world: &mut World, lets: String, add: String) {
    insert_miras_habit(world, &format!("    let: {{ {lets} }}\n"), &add);
}

/// Insert Mira's habit `toss`, with its `lets` line, adding `add` to her vigor.
fn insert_miras_habit(world: &World, lets: &str, add: &str) {
    let pack = host::pack(world).to_path_buf();
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read mira: {err}"));
    let next = text.replacen(
        "rules:\n  - id: leave_door_note\n",
        &format!(
            "rules:\n  - id: toss\n    when: habit\n{lets}    do:\n      - meter:\n          of: self\n          key: vigor\n          add: \"{add}\"\n  - id: leave_door_note\n"
        ),
        1,
    );
    assert_ne!(next, text, "mira habit was not inserted");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write mira: {err}"));
    crate::support::stamp_pack_rev(&pack);
}

#[when(expr = "the GM queues Aldric's wait and commits")]
async fn the_gm_queues_aldrics_wait_and_commits(world: &mut World) {
    let pack = host::pack(world).to_path_buf();
    host::listen(world, &pack).await;
    host::seat_gm(world).await;
    host::gm_tool_call(world, 6, "queue", r#"{"actor":"aldric","verb":"wait"}"#).await;
    world.stdout = host::gm_tool_call(world, 7, "commit", r#"{"include_round":true}"#).await;
}

#[when(
    regex = r#"^the pack's hit stores "(.+)" and has "(.+)" as (?:its|a) (vigor add|nested check's vigor add|condition value|cost amount|second let)$"#
)]
fn the_packs_hit_stores_and_has(world: &mut World, lets: String, number: String, field: String) {
    append_hit_line(world, &format!("let: {{ {lets} }}\n    "), &number, &field);
}

#[when(expr = "the pack's hit stores {string} and adds {string} to Rowan's and Mira's vigor")]
fn the_packs_hit_stores_and_adds_twice(world: &mut World, lets: String, add: String) {
    append_rule(
        world,
        &format!(
            "\n  - id: hit\n    when: {{ verb: hit }}\n    let: {{ {lets} }}\n    do:\n      - meter: {{ of: self, key: vigor, add: {add} }}\n      - meter: {{ of: mira, key: vigor, add: {add} }}\n"
        ),
    );
}

#[when(
    expr = "the pack's hit stores {string}, checks {string} against {int}, and adds {string} to vigor on a success"
)]
fn the_packs_hit_stores_and_checks(
    world: &mut World,
    lets: String,
    total: String,
    success_at: i64,
    add: String,
) {
    append_rule(
        world,
        &format!(
            "\n  - id: hit\n    when: {{ verb: hit }}\n    let: {{ {lets} }}\n    check: {{ total: {total}, success_at: {success_at} }}\n    success:\n      - meter: {{ of: self, key: vigor, add: {add} }}\n"
        ),
    );
}

#[when(expr = "the pack's hit stores {string}, takes {int} ticks, and adds {string} to vigor")]
fn the_packs_timed_hit_stores(world: &mut World, lets: String, ticks: u64, add: String) {
    append_rule(
        world,
        &format!(
            "\n  - id: hit\n    when: {{ verb: hit }}\n    let: {{ {lets} }}\n    cost: {{ ticks: {ticks} }}\n    do:\n      - meter: {{ of: self, key: vigor, add: {add} }}\n"
        ),
    );
}

#[when(expr = "Mira's rouse takes {int} ticks and adds {int} to Rowan's rest")]
fn miras_rouse(world: &mut World, ticks: u64, add: i64) {
    append_rule(
        world,
        &format!(
            "\n  - id: rouse\n    when: {{ verb: rouse }}\n    cost: {{ ticks: {ticks} }}\n    do:\n      - meter: {{ of: rowan, key: rest, add: {add} }}\n"
        ),
    );
}

#[given(expr = "the rule \"stock\" reads Rowan's rest of 3 in every number")]
fn stock_reads_rest(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    super::modifiers::add_modifier(&pack, "fatigue", "vigor", "");
    crate::support::append(
        &pack.join("rules/verbs.yaml"),
        r#"
  - id: stock
    when: { verb: stock }
    if:
      - has_item: { subject: self, id: lamp_oil, count: "self.rest - 3" }
      - players: { op: ">=", value: "self.rest - 1" }
      - cycle: { id: day, from: "self.rest * 300", to: "self.rest * 400" }
    cost: { ticks: "self.rest + 7" }
    do:
      - item: { of: self, id: chock, add: "self.rest * 2", max: "self.rest + 1" }
      - modifier: { to: self, id: fatigue, add: -1, ticks: "self.rest + 2" }
"#,
    );
    crate::support::stamp_pack_rev(&pack);
}

#[given(
    regex = r#"^the pack has "(.+)" as (the hit's (?:condition value|cost amount|cost item count|cost ticks)|the street exit's ticks|the strain's modifier ticks|the strain's cycle bound)$"#
)]
fn the_pack_has(world: &mut World, number: String, field: String) {
    let pack = world.pack.clone().unwrap();
    let verbs = pack.join("rules/verbs.yaml");
    let strain = "\n  - id: strain\n    when: { verb: strain }\n";
    if let Some(hit_field) = field.strip_prefix("the hit's ") {
        return append_hit_line(world, "", &number, hit_field);
    }
    match field.as_str() {
        "the street exit's ticks" => {
            let cost = format!("    cost: {{ ticks: \"{number}\" }}\n");
            crate::support::split_path(&pack, ["docks_shop", "docks_street"], [&cost, ""]);
        }
        "the strain's modifier ticks" => {
            super::modifiers::add_modifier(&pack, "fatigue", "vigor", "");
            let modifier = format!("{{ to: self, id: fatigue, add: -1, ticks: \"{number}\" }}");
            crate::support::append(&verbs, &format!("{strain}    do: [modifier: {modifier}]\n"));
        }
        _ => {
            let cycle = format!("{{ id: day, from: \"{number}\", to: 0 }}");
            crate::support::append(
                &verbs,
                &format!("{strain}    if: [cycle: {cycle}]\n    do: []\n"),
            );
        }
    }
    crate::support::stamp_pack_rev(&pack);
}

#[when(
    regex = r"^the player (hits|hits Aldric|hits while the GM queues Mira's rouse) and the GM commits$"
)]
async fn the_player_hits_and_the_gm_commits(world: &mut World, hits: String) {
    let pack = host::pack(world).to_path_buf();
    host::listen(world, &pack).await;
    host::seat_rowan(world, &pack).await;
    host::seat_gm(world).await;
    let line = if hits == "hits Aldric" {
        "hit aldric"
    } else {
        "hit"
    };
    let _ = host::player_send(world, line, "\"kind\":\"queued\"").await;
    if hits.ends_with("rouse") {
        let queued =
            host::gm_tool_call(world, 6, "queue", r#"{"actor":"mira","verb":"rouse"}"#).await;
        assert!(!queued.contains("\"refused\""), "{queued}");
    }
    world.mark = host::player_mark(world);
    world.stdout = host::gm_tool_call(world, 7, "commit", r#"{"include_round":true}"#).await;
}

#[then(expr = "Rowan's vigor is {int}")]
fn rowans_vigor_is(world: &mut World, expected: i64) {
    assert_eq!(rowans_vigor(world), expected);
}

#[then(expr = "Mira's vigor rose by the 1d4 face")]
fn miras_vigor_rose_by_the_1d4_face(world: &mut World) {
    let faces = dice_faces(world, "1d4");
    assert_eq!(faces.len(), 1, "{faces:?}");
    assert_eq!(vigor_of(world, "mira"), 1 + i64::from(faces[0]));
}

#[then(expr = "the report names the line holding {string}")]
fn the_report_names_the_line_holding(world: &mut World, needle: String) {
    the_report_names_the_line_in_file_holding(world, "rules/verbs.yaml".into(), needle);
}

/// The report locates the last line of the pack's `file` that holds `needle`.
#[then(expr = "the report names the line in {word} holding {string}")]
fn the_report_names_the_line_in_file_holding(world: &mut World, file: String, needle: String) {
    let pack = host::pack(world);
    let text =
        fs::read_to_string(pack.join(&file)).unwrap_or_else(|err| panic!("read {file}: {err}"));
    let mut line = None;
    for (index, item) in text.lines().enumerate() {
        if item.contains(&needle) {
            line = Some(index + 1);
        }
    }
    let line = line.unwrap_or_else(|| panic!("no line holding {needle}"));
    let located = format!("{file}:{line}:");
    assert!(
        world.stderr.contains(&located),
        "missing {located}\n{}",
        world.stderr
    );
}

#[then(expr = "Rowan's vigor rose by the 2d6 faces")]
fn rowans_vigor_rose_by_the_2d6_faces(world: &mut World) {
    let faces = dice_faces(world, "2d6");
    let sum: i64 = faces.iter().map(|face| i64::from(*face)).sum();
    assert_eq!(faces.len(), 2, "{faces:?}");
    assert_eq!(rowans_vigor(world), 1 + sum);
}

#[then(expr = "Rowan's vigor rose by the {word} face of {word}")]
fn rowans_vigor_rose_by_the_kept_face(world: &mut World, kept: String, dice: String) {
    let faces = dice_faces(world, &dice).into_iter();
    let face = if kept == "higher" {
        faces.max()
    } else {
        faces.min()
    };
    assert_eq!(rowans_vigor(world), 1 + i64::from(face.unwrap_or(0)));
}

#[then(expr = "the GM commit reply shows the 2d6 faces")]
fn the_gm_commit_reply_shows_the_2d6_faces(world: &mut World) {
    assert!(
        world.stdout.contains("2d6 →"),
        "missing dice\n{}",
        world.stdout
    );
}

#[then(expr = "Rowan's and Mira's vigor both rose by the boost total")]
fn rowans_and_miras_vigor_rose_by_the_boost_total(world: &mut World) {
    let line = dice_line(world);
    assert!(line.starts_with("boost: 2d6 → "), "{line}");
    let total: i64 = (line.rsplit(" = ").next())
        .and_then(|total| total.parse().ok())
        .unwrap_or_else(|| panic!("no total\n{line}"));
    assert_eq!(rowans_vigor(world), 1 + total);
    assert_eq!(vigor_of(world, "mira"), 1 + total);
}

#[then(expr = "the GM commit reply holds dice lines for {string}")]
fn the_gm_commit_reply_holds_dice_lines(world: &mut World, expected: String) {
    let reply = host::read_reply(&world.stdout);
    let dice = &reply["dice"];
    let lines = dice.as_array().unwrap_or_else(|| panic!("no dice\n{dice}"));
    let prefixes: Vec<&str> = expected.split(", ").collect();
    assert_eq!(lines.len(), prefixes.len(), "{dice}");
    for (line, prefix) in lines.iter().zip(prefixes) {
        let line = line.as_str().unwrap_or_default();
        assert!(line.starts_with(&format!("{prefix} → ")), "{dice}");
    }
}

#[then(expr = "the player text has no dice faces")]
async fn the_player_text_has_no_dice_faces(world: &mut World) {
    let added = host::wait_player_from(world, world.mark, "\"kind\":").await;
    assert!(!added.contains('→'), "{added}");
    assert!(!added.contains("2d6"), "{added}");
}

#[then(expr = "the report names the expression line")]
fn the_report_names_the_expression_line(world: &mut World) {
    let pack = host::pack(world);
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

/// Append rule `hit`, its `lets` line, then `number` as `field`.
fn append_hit_line(world: &World, lets: &str, number: &str, field: &str) {
    let lines = match field {
        "condition value" => format!(
            "if: [meter: {{ of: self, id: vigor, op: \">=\", value: \"{number}\" }}]\n    do: []"
        ),
        "cost amount" => format!("cost: {{ meters: {{ vigor: \"{number}\" }} }}\n    do: []"),
        "cost item count" => format!("cost: {{ items: {{ chock: \"{number}\" }} }}\n    do: []"),
        "cost ticks" => format!("cost: {{ ticks: \"{number}\" }}\n    do: []"),
        "second let" => format!("let: {{ {number} }}\n    do: []"),
        "nested check's vigor add" => format!(
            "do: [check: {{ by: target, total: \"{number}\", success_at: \"{number}\", success: [meter: {{ of: self, key: vigor, add: \"{number}\" }}] }}]"
        ),
        _ => format!("do: [meter: {{ of: self, key: vigor, add: \"{number}\" }}]"),
    };
    let target = match field {
        "nested check's vigor add" => ", target: actor",
        _ => "",
    };
    let rule = format!("\n  - id: hit\n    when: {{ verb: hit{target} }}\n    {lets}{lines}\n");
    append_rule(world, &rule);
}

/// Append `rule` to the pack's verbs and restamp its save.
fn append_rule(world: &World, rule: &str) {
    let pack = host::pack(world).to_path_buf();
    crate::support::append(&pack.join("rules/verbs.yaml"), rule);
    crate::support::stamp_pack_rev(&pack);
}

fn append_hit(world: &World, of: &str, add: &str) {
    append_rule(
        world,
        &format!(
            "\n  - id: hit\n    when:\n      verb: hit\n    do:\n      - meter:\n          of: {of}\n          key: vigor\n          add: {add}\n"
        ),
    );
}

fn rowans_vigor(world: &World) -> i64 {
    vigor_of(world, "rowan")
}

fn vigor_of(world: &World, actor: &str) -> i64 {
    let now = world_of(host::pack(world));
    let id = actor.into();
    let meter = now
        .actors()
        .get::<component::Meter>(&id)
        .get_or_base("vigor")
        .unwrap_or_else(|| panic!("{actor}'s vigor missing"));
    meter.base + meter.offset
}

/// The first dice line in the GM's commit reply.
fn dice_line(world: &World) -> String {
    let reply = host::read_reply(&world.stdout);
    let line = reply["dice"][0].as_str();
    line.unwrap_or_else(|| panic!("no dice line\n{}", world.stdout))
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
