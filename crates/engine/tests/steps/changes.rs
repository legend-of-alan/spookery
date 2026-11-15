use std::fs;

use cucumber::{given, then, when};
use spookery_client::text;
use spookery_engine::testing::{self, stamp_pack_rev};
use spookery_engine::World as EngineWorld;
use spookery_engine::{component, Meter, Presence};
use spookery_protocol::ServerBody;

use crate::{split_line, World};

const SECRET_ID: &str = "fact_sealed_name";
const SECRET_TEXT: &str = "the sealed name";

#[given(expr = "harbormouse has a hidden meter")]
fn harbormouse_has_a_hidden_meter(world: &mut World) {
    let pack = world.pack();
    let mut meters = fs::read_to_string(pack.join("rules/meters.yaml")).unwrap();
    meters.push_str(
        "  - id: omen\n    min: 0\n    base_cap: 5\n    max: 5\n    base: 1\n    hidden: true\n",
    );
    fs::write(pack.join("rules/meters.yaml"), meters).unwrap();
    stamp_pack_rev(pack);
    spookery_engine::migrate_pack(pack).unwrap_or_else(|err| panic!("migrate: {err}"));
}

#[given(expr = "a round that changes a hidden fact")]
fn a_round_that_changes_a_hidden_fact(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let lane = pack.join("world/places/mill_lane.md");
    let mut place = fs::read_to_string(&lane).unwrap();
    let from = "facts: []\n";
    assert!(place.contains(from), "{place}");
    let fact = format!("facts:\n  - id: {SECRET_ID}\n    text: {SECRET_TEXT}\n    hidden: true\n");
    place = place.replacen(from, &fact, 1);
    fs::write(lane, place).unwrap();
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(&format!(
        "  - id: spill\n    when:\n      verb: spill\n    do:\n      - reveal:\n          to: briar\n          fact: {SECRET_ID}\n      - flag:\n          of: world\n          key: {SECRET_ID}\n          value: true\n      - meter:\n          of: briar\n          key: hp\n          add: -1\n"
    ));
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
    stamp_pack_rev(&pack);
    let mut opened = testing::resume(&pack);
    let output = play(&mut opened, "spill");
    world.summary_lines = output.changed_keys.clone();
    world.seen = change_line(&output);
}

#[when(expr = "harbormouse's might is raised and the seeded strike succeeds")]
fn harbormouses_might_is_raised(world: &mut World) {
    let pack = world.pack().to_path_buf();
    testing::edit_save(&pack, |state| {
        let might = Meter {
            base: 40,
            offset: 0,
        };
        let mut actors = state.actors_mut();
        let harbormouse = "harbormouse".into();
        actors
            .get_mut::<Meter>(&harbormouse)
            .set("might", Some(might))
            .unwrap();
    });
    let mut opened = testing::resume(&pack);
    let output = play(&mut opened, "strike briar");
    world.seen = change_line(&output);
}

#[when(expr = "harbormouse uses a tonic")]
fn harbormouse_uses_a_tonic(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut opened = testing::resume(&pack);
    let output = play(&mut opened, "use tonic");
    world.seen = change_line(&output);
}

#[when(expr = "harbormouse uses the tonic while someone out of sight gains one")]
fn harbormouse_uses_the_tonic_while_bram_gains_one(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push('\n');
    verbs.push_str(
        "  - id: sip\n    when:\n      verb: sip\n    cost:\n      items:\n        tonic: 1\n    do:\n      - item:\n          of: bram\n          id: tonic\n          add: 1\n",
    );
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
    stamp_pack_rev(&pack);
    let mut opened = testing::resume(&pack);
    let output = play(&mut opened, "sip");
    world.summary_lines = output.changed_keys.clone();
    world.seen = change_line(&output);
}

#[then(expr = "that line does not mention the unseen gain")]
fn the_line_skips_the_unseen_gain(world: &mut World) {
    assert!(
        world
            .summary_lines
            .iter()
            .any(|item| item.contains("actor_item bram tonic")),
        "{:?}",
        world.summary_lines
    );
    let line = one_line(&world.seen);
    let lower = line.to_lowercase();
    assert!(lower.contains("used up"), "{line}");
    assert!(!lower.contains("lost"), "{line}");
    assert!(!lower.contains("gained"), "{line}");
    assert!(!lower.contains("bram"), "{line}");
}

#[when(expr = "harbormouse drops the tonic")]
fn harbormouse_drops_the_tonic(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut opened = testing::resume(&pack);
    let output = play(&mut opened, "drop tonic");
    world.seen = change_line(&output);
}

#[when(expr = "harbormouse climbs to the loft")]
fn harbormouse_climbs_to_the_loft(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut opened = testing::resume(&pack);
    let output = play(&mut opened, "travel loft");
    world.seen = change_line(&output);
}

#[then(expr = "harbormouse sees one line naming the Loft as their place")]
fn harbormouse_sees_the_loft_named_as_their_place(world: &mut World) {
    let line = one_line(&world.seen);
    assert_eq!(line, "Your place is now Loft.");
}

#[given(expr = "a tonic is on the ground")]
fn a_tonic_is_on_the_ground(world: &mut World) {
    let pack = world.pack().to_path_buf();
    testing::edit_save(&pack, |state| {
        state
            .places_mut()
            .get_mut::<component::Item>(&"gate_yard".into())
            .shift("tonic", 1, None)
            .unwrap();
    });
}

#[when(expr = "harbormouse takes the tonic")]
fn harbormouse_takes_the_tonic(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut opened = testing::resume(&pack);
    let output = play(&mut opened, "take tonic");
    world.seen = change_line(&output);
}

#[when(expr = "harbormouse looks at their view")]
fn harbormouse_looks_at_their_view(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let opened = testing::resume(&pack);
    world.seen = meter_lines(&opened);
}

#[then(expr = "harbormouse's change line does not name briar's hp")]
fn harbormouse_does_not_see_briars_hp(world: &mut World) {
    let lower = world.seen.to_lowercase();
    assert!(!lower.contains("hp"), "{}", world.seen);
}

#[then(expr = "harbormouse sees one line saying the tonic was used up")]
fn harbormouse_sees_the_tonic_used_up(world: &mut World) {
    let line = one_line(&world.seen);
    let lower = line.to_lowercase();
    assert!(
        lower.contains("tonic") && lower.contains("used up"),
        "{line}"
    );
}

#[then(expr = "harbormouse sees one line saying they dropped the tonic")]
fn harbormouse_sees_they_dropped_the_tonic(world: &mut World) {
    let line = one_line(&world.seen);
    assert_eq!(line, "You dropped the Bitter tonic.");
}

#[then(expr = "harbormouse sees one line saying they picked up the tonic")]
fn harbormouse_sees_they_picked_up_the_tonic(world: &mut World) {
    let line = one_line(&world.seen);
    assert_eq!(line, "You picked up the Bitter tonic.");
}

#[then(expr = "harbormouse sees their own hp and might")]
fn harbormouse_sees_hp_and_might(world: &mut World) {
    assert!(world.seen.contains("hp: 8"), "{}", world.seen);
    assert!(world.seen.contains("might: 30"), "{}", world.seen);
    assert!(!world.seen.contains("might: 2"), "{}", world.seen);
}

#[then(expr = "harbormouse does not see a hidden meter")]
fn harbormouse_does_not_see_a_hidden_meter(world: &mut World) {
    assert!(
        !world.seen.to_lowercase().contains("omen"),
        "{}",
        world.seen
    );
}

#[then(expr = "the player's change line does not mention it")]
fn the_change_line_hides_the_fact(world: &mut World) {
    assert!(
        world
            .summary_lines
            .iter()
            .any(|item| item.contains(SECRET_ID)),
        "{:?}",
        world.summary_lines
    );
    assert!(
        world
            .summary_lines
            .iter()
            .any(|item| item.contains("actor_meter briar hp")),
        "{:?}",
        world.summary_lines
    );
    let line = &world.seen;
    assert!(!line.to_lowercase().contains("hp"), "{line}");
    assert!(!line.contains(SECRET_ID), "{line}");
    assert!(!line.contains(SECRET_TEXT), "{line}");
    assert!(!line.contains("world_flag"), "{line}");
}

fn one_line(text: &str) -> &str {
    assert!(!text.is_empty() && !text.contains('\n'), "{text}");
    text
}

/// Harbormouse is the one seated player.
fn harbormouse() -> Presence {
    Presence {
        players: 1,
        actors_present: vec!["harbormouse".into()],
    }
}

fn play(world: &mut EngineWorld, line: &str) -> spookery_engine::RoundOutput {
    let (verb, target) = split_line(line);
    let wall = world.state().last_round_at.clone();
    world
        .queue_player_move(&harbormouse(), &"harbormouse".into(), verb, target)
        .unwrap_or_else(|err| panic!("{line}: {}", err.code.as_str()));
    let outputs = world
        .commit_then_fast_forward(&harbormouse(), &wall)
        .unwrap_or_else(|err| panic!("{line}: {err}"));
    outputs.into_iter().next().unwrap()
}

fn change_line(output: &spookery_engine::RoundOutput) -> String {
    output
        .seen
        .iter()
        .find(|seen| seen.viewer == *"harbormouse")
        .map(|seen| {
            text::render(&ServerBody::Changes {
                viewer: "harbormouse".into(),
                changes: seen.changes.clone(),
            })
        })
        .unwrap_or_default()
}

fn meter_lines(world: &EngineWorld) -> String {
    let view = world
        .perceive(&harbormouse(), "harbormouse")
        .unwrap_or_else(|err| panic!("perceive: {}", err.as_str()));
    view.meters
        .iter()
        .map(|meter| format!("{}: {}", meter.id, meter.value))
        .collect::<Vec<_>>()
        .join("\n")
}

#[when(expr = "harbormouse plays a juggle that gains a tonic and gives it back")]
fn harbormouse_juggles_a_tonic(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push('\n');
    verbs.push_str(
        "  - id: juggle\n    when:\n      verb: juggle\n    do:\n      - item:\n          of: harbormouse\n          id: tonic\n          add: 1\n      - item:\n          of: harbormouse\n          id: tonic\n          add: -1\n",
    );
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
    stamp_pack_rev(&pack);
    let mut opened = testing::resume(&pack);
    let output = play(&mut opened, "juggle");
    world.summary_lines = output.changed_keys.clone();
    world.seen = change_line(&output);
}

#[then(expr = "the round names no change to harbormouse's tonic")]
fn no_change_to_the_tonic(world: &mut World) {
    let named = |item: &String| item.contains("actor_item harbormouse tonic");
    assert!(
        !world.summary_lines.iter().any(named),
        "{:?}",
        world.summary_lines
    );
    assert!(
        !world.seen.to_lowercase().contains("tonic"),
        "{}",
        world.seen
    );
}
