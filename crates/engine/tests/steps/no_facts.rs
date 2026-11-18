//! A place with no facts still accepts moves and habits.

use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing as support;
use spookery_engine::{GlobalId, Note, Presence};

use crate::{resolve_id, split_line, World};

#[when(expr = "Harbormouse plays {string}")]
fn harbormouse_plays(world: &mut World, line: String) {
    play(world, "harbormouse", &line);
}

#[then(regex = r#"^(?P<actor>\S+) is in (?P<place>\S+|"[^"]+")$"#)]
fn is_in(world: &mut World, actor: String, place: String) {
    let now = support::world_of(world.pack());
    let [actor, place] =
        [&actor, &place].map(|text| resolve_id(now.canon(), text.trim_matches('"')));
    assert_eq!(support::place_of(&now, &actor), place);
}

#[then(expr = "the wait lands with nothing refused")]
fn the_wait_lands_with_nothing_refused(world: &mut World) {
    assert_eq!(world.scratch, "", "refused");
    let now = support::world_of(world.pack());
    assert_eq!(support::act_of(&now, "harbormouse").last_act_round, 1);
}

#[given(expr = "Mira stands in a place with no facts")]
fn mira_stands_in_a_place_with_no_facts(world: &mut World) {
    // repeats module: mirrors its step text
    let pack = support::copy_pack("docks", "no-facts");
    strip_room_facts(&pack);
    support::edit_save(&pack, |ecs_world| {
        // No writer forgets a fact, so Mira's come off the save as the file holds it.
        let mut save = serde_json::to_value(&**ecs_world).unwrap();
        if let Some(known) = save["actors"]["known_facts"].as_object_mut() {
            known.remove("mira");
        }
        **ecs_world = serde_json::from_value(save).unwrap();
    });
    let opened = spookery_engine::Session::open(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    let view = opened
        .perceive(&Presence::default(), "mira")
        .unwrap_or_else(|err| panic!("perceive: {err}"));
    assert!(
        view.known_facts.is_empty(),
        "Mira still sees {:?}",
        view.known_facts
            .iter()
            .map(|fact| fact.id.as_str())
            .collect::<Vec<_>>()
    );
    world.pack = Some(pack);
}

#[when(expr = "a docks round plays")]
fn a_docks_round_plays(world: &mut World) {
    let pack = world.pack();
    spookery_engine::play_pack_once(pack).unwrap_or_else(|err| panic!("round: {err}"));
}

#[then(expr = "Mira's habit fires")]
fn miras_habit_fires(world: &mut World) {
    let pack = world.pack();
    let ecs_world = support::world_of(pack);
    let notes = ecs_world.globals().get::<Note>(&GlobalId);
    assert!(
        notes.contains("note_mira_door"),
        "the door note was not stored"
    );
}

fn play(world: &mut World, actor: &str, line: &str) {
    let pack = world.pack();
    let lock = spookery_engine::lock_pack(pack).unwrap();
    let mut opened =
        spookery_engine::Session::resume(pack, lock).unwrap_or_else(|err| panic!("resume: {err}"));
    let seated = Presence {
        players: 1,
        actors_present: vec![actor.into()],
    };
    let (verb, target) = split_line(line);
    let wall = opened.world().last_round_at.clone();
    opened
        .queue_player_move(&seated, &actor.into(), verb, target)
        .unwrap_or_else(|err| panic!("{line}: {}", err.code.as_str()));
    let outputs = opened
        .commit_then_fast_forward(&seated, &wall)
        .unwrap_or_else(|err| panic!("{line}: {err}"));
    let reports = outputs.iter().flat_map(|output| &output.reports);
    world.scratch = super::round_play::list_refusals(reports);
}

fn strip_room_facts(pack: &Path) {
    let path = pack.join("world/places/mira_rooms.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read rooms: {err}"));
    let unix = text.replace("\r\n", "\n");
    let from = "facts:\n  - id: fact_room_lamp\n    text: A lamp burns in the room.\n";
    assert!(unix.contains(from), "room facts were not found\n{unix}");
    fs::write(&path, unix.replace(from, "facts: []\n"))
        .unwrap_or_else(|err| panic!("write rooms: {err}"));
}
