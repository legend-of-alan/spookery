use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::model::Presence;
use spookery_engine::World as Shard;

use crate::World;
use spookery_engine::testing as support;
use spookery_engine::testing::stamp_pack_rev;

const SECRET_ID: &str = "fact_sealed_name";
const SECRET_TEXT: &str = "the sealed name";

#[given(expr = "sample-d20 with player {string} seated facing {string}")]
fn sample_d20_with_lark_facing_briar(world: &mut World, player: String, facing: String) {
    assert_eq!(player, "lark");
    assert_eq!(facing, "briar");
    world.pack = Some(support::copy_pack("sample-d20", "matters"));
}

#[given(expr = "lark has a hidden meter")]
fn lark_has_a_hidden_meter(world: &mut World) {
    let pack = world.pack();
    let mut meters = fs::read_to_string(pack.join("rules/meters.yaml")).unwrap();
    meters.push_str(
        "  - id: omen\n    min: 0\n    base_cap: 5\n    max: 5\n    start: 1\n    hidden: true\n",
    );
    fs::write(pack.join("rules/meters.yaml"), meters).unwrap();
    stamp_pack_rev(pack);
    spookery_engine::migrate_pack(pack).unwrap_or_else(|err| panic!("migrate: {err}"));
}

#[given(expr = "a round that changes a hidden fact")]
fn a_round_that_changes_a_hidden_fact(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut cast = fs::read_to_string(pack.join("cast/briar.md")).unwrap();
    let from = "facts: []\n";
    assert!(cast.contains(from), "{cast}");
    let fact = format!("facts:\n  - id: {SECRET_ID}\n    text: {SECRET_TEXT}\n    hidden: true\n");
    cast = cast.replacen(from, &fact, 1);
    fs::write(pack.join("cast/briar.md"), cast).unwrap();
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(&format!(
        "  - id: spill\n    when:\n      verb: spill\n    do:\n      - reveal:\n          who: npc_briar\n          fact: {SECRET_ID}\n      - flag:\n          who: world\n          key: {SECRET_ID}\n          value: true\n      - meter:\n          who: npc_briar\n          key: hp\n          add: -1\n"
    ));
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
    stamp_pack_rev(&pack);
    let mut shard = seat(&pack);
    let output = play(&mut shard, "spill");
    world.summary = output.summary.clone();
    world.stdout = change_line(&output);
}

#[when(expr = "lark's might is raised and the seeded strike succeeds")]
fn larks_might_is_raised(world: &mut World) {
    let pack = world.pack().to_path_buf();
    support::edit_save(&pack, |now| {
        let might = now
            .actors
            .get_mut("pc_lark")
            .unwrap()
            .meters_mut()
            .get_mut("might")
            .unwrap();
        might.base = 40;
        might.offset = 0;
    });
    let mut shard = seat(&pack);
    let output = play(&mut shard, "strike npc_briar");
    world.stdout = change_line(&output);
}

#[when(expr = "lark uses a tonic")]
fn lark_uses_a_tonic(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut shard = seat(&pack);
    let output = play(&mut shard, "use tonic");
    world.stdout = change_line(&output);
}

#[when(expr = "lark uses her tonic while someone out of sight gains one")]
fn lark_uses_her_tonic_while_oden_gains_one(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push('\n');
    verbs.push_str(
        "  - id: sip\n    when:\n      verb: sip\n    cost:\n      items:\n        tonic: 1\n    do:\n      - item:\n          who: npc_oden\n          id: tonic\n          add: 1\n",
    );
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
    stamp_pack_rev(&pack);
    let mut shard = seat(&pack);
    let output = play(&mut shard, "sip");
    world.summary = output.summary.clone();
    world.stdout = change_line(&output);
}

#[then(expr = "that line does not mention the unseen gain")]
fn the_line_skips_the_unseen_gain(world: &mut World) {
    assert!(
        world
            .summary
            .iter()
            .any(|item| item.contains("actor_item npc_oden tonic")),
        "{:?}",
        world.summary
    );
    let line = one_line(&world.stdout);
    let lower = line.to_lowercase();
    assert!(lower.contains("used up"), "{line}");
    assert!(!lower.contains("lost"), "{line}");
    assert!(!lower.contains("gained"), "{line}");
    assert!(!lower.contains("oden"), "{line}");
}

#[when(expr = "lark drops the tonic")]
fn lark_drops_the_tonic(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut shard = seat(&pack);
    let output = play(&mut shard, "drop tonic");
    world.stdout = change_line(&output);
}

#[given(expr = "a tonic is on the ground")]
fn a_tonic_is_on_the_ground(world: &mut World) {
    let pack = world.pack().to_path_buf();
    support::edit_save(&pack, |now| {
        now.places
            .get_mut("gate_yard")
            .unwrap()
            .items
            .insert("tonic".to_string(), 1);
    });
}

#[when(expr = "lark takes the tonic")]
fn lark_takes_the_tonic(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let mut shard = seat(&pack);
    let output = play(&mut shard, "take tonic");
    world.stdout = change_line(&output);
}

#[when(expr = "lark looks at their view")]
fn lark_looks_at_their_view(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let shard = seat(&pack);
    world.stdout = shown_text(&shard);
}

#[then(expr = "lark sees one line naming the change to briar's hp")]
fn lark_sees_briars_hp_change(world: &mut World) {
    let line = one_line(&world.stdout);
    let lower = line.to_lowercase();
    assert!(lower.contains("briar") && lower.contains("hp"), "{line}");
    assert!(!line.contains("actor_meter"), "{line}");
    assert!(line.contains('4'), "{line}");
}

#[then(expr = "lark sees one line saying the tonic was used up")]
fn lark_sees_the_tonic_used_up(world: &mut World) {
    let line = one_line(&world.stdout);
    let lower = line.to_lowercase();
    assert!(
        lower.contains("tonic") && lower.contains("used up"),
        "{line}"
    );
}

#[then(expr = "lark sees one line saying she dropped the tonic")]
fn lark_sees_she_dropped_the_tonic(world: &mut World) {
    let line = one_line(&world.stdout);
    assert_eq!(line, "You dropped the Bitter tonic.");
}

#[then(expr = "lark sees one line saying she picked up the tonic")]
fn lark_sees_she_picked_up_the_tonic(world: &mut World) {
    let line = one_line(&world.stdout);
    assert_eq!(line, "You picked up the Bitter tonic.");
}

#[then(expr = "lark sees lark's hp and might")]
fn lark_sees_hp_and_might(world: &mut World) {
    assert!(world.stdout.contains("hp: 8"), "{}", world.stdout);
    assert!(world.stdout.contains("might: 30"), "{}", world.stdout);
    assert!(!world.stdout.contains("might: 2"), "{}", world.stdout);
}

#[then(expr = "lark does not see a hidden meter")]
fn lark_does_not_see_a_hidden_meter(world: &mut World) {
    assert!(
        !world.stdout.to_lowercase().contains("omen"),
        "{}",
        world.stdout
    );
}

#[then(expr = "the player's change line does not mention it")]
fn the_change_line_hides_the_fact(world: &mut World) {
    assert!(
        world.summary.iter().any(|item| item.contains(SECRET_ID)),
        "{:?}",
        world.summary
    );
    assert!(
        world
            .summary
            .iter()
            .any(|item| item.contains("actor_meter npc_briar hp")),
        "{:?}",
        world.summary
    );
    let line = one_line(&world.stdout);
    let lower = line.to_lowercase();
    assert!(lower.contains("briar") && lower.contains("hp"), "{line}");
    assert!(line.contains('7'), "{line}");
    assert!(!line.contains(SECRET_ID), "{line}");
    assert!(!line.contains(SECRET_TEXT), "{line}");
    assert!(!line.contains("world_flag"), "{line}");
}

fn one_line(text: &str) -> &str {
    assert!(!text.is_empty() && !text.contains('\n'), "{text}");
    text
}

fn seat(pack: &Path) -> Shard {
    let mut world = match Shard::open(pack) {
        Ok(world) => world,
        Err(err) => {
            let report = spookery_engine::check_pack(pack)
                .iter()
                .map(|diag| diag.report_line())
                .collect::<Vec<_>>()
                .join("\n");
            panic!("open: {}\n{report}", err.token());
        }
    };
    world.set_presence(Presence {
        humans: 1,
        actors_present: vec!["pc_lark".to_string()],
    });
    world
}

fn play(world: &mut Shard, line: &str) -> spookery_engine::RoundOutput {
    let (verb, target) = support::split_move(line);
    let wall = world.now().last_tick_at.clone();
    match spookery_engine::handle_player_move(world, "pc_lark", verb, target, false, &wall) {
        Ok(spookery_engine::MoveOutcome::Committed { outputs, .. }) => {
            outputs.into_iter().next().unwrap()
        }
        Ok(_) => panic!("{line} did not commit"),
        Err(err) => panic!("{line}: {}", err.token()),
    }
}

fn change_line(output: &spookery_engine::RoundOutput) -> String {
    output
        .seen
        .iter()
        .find(|seen| seen.viewer == "pc_lark")
        .map(|seen| seen.line.clone())
        .unwrap_or_default()
}

fn shown_text(world: &Shard) -> String {
    let view = world
        .perceive("pc_lark")
        .unwrap_or_else(|err| panic!("perceive: {}", err.token()));
    view.meters
        .iter()
        .map(|meter| format!("{}: {}", meter.id, meter.value))
        .collect::<Vec<_>>()
        .join("\n")
}
