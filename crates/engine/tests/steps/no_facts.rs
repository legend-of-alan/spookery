//! A place with no facts still accepts moves and habits.

use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing::{self as support, journal::list_round_keys};
use spookery_engine::Presence;

use crate::World;

#[given(expr = "a copy of the sample-d20 pack")]
fn a_copy_of_the_sample_d20_pack(world: &mut World) {
    world.pack = Some(support::copy_pack("sample-d20", "sample"));
}

#[when(expr = "Lark plays {string}")]
fn lark_plays(world: &mut World, line: String) {
    play(world, "pc_lark", &line);
}

#[then(expr = "the journal commits two location effects for Lark")]
fn two_location_effects_for_lark(world: &mut World) {
    let moves = location_effects(world.pack(), "pc_lark");
    assert_eq!(
        moves,
        vec![
            ("gate_yard".to_string(), "loft".to_string()),
            ("loft".to_string(), "gate_yard".to_string()),
        ],
        "{moves:?}"
    );
}

#[then(expr = "Lark is in the Gate Yard")]
fn lark_is_in_the_gate_yard(world: &mut World) {
    assert_eq!(place_of(world.pack(), "pc_lark"), "gate_yard");
}

#[when(expr = "Nell waits")]
fn nell_waits(world: &mut World) {
    play(world, "pc_nell", "wait");
}

#[then(expr = "the wait commits with nothing rejected")]
fn the_wait_commits_with_nothing_rejected(world: &mut World) {
    let pack = world.pack();
    let mut found = false;
    for key in list_round_keys(pack) {
        let round = support::read_round(pack, &key);
        let waited = round["intents"].as_array().is_some_and(|intents| {
            intents
                .iter()
                .any(|intent| intent["move"] == "wait" && intent["actor"] == "player:pc_nell")
        });
        if !waited {
            continue;
        }
        found = true;
        let rejected = round["refused"].as_array().unwrap();
        assert!(rejected.is_empty(), "{key}: {rejected:?}");
    }
    assert!(found, "Nell's wait was not committed");
}

#[given(expr = "Mira stands in a place with no facts")]
fn mira_stands_in_a_place_with_no_facts(world: &mut World) {
    let pack = support::copy_docks("no-facts");
    strip_room_facts(&pack);
    support::edit_save(&pack, |now| {
        now.actors
            .get_mut("mira")
            .unwrap()
            .known_facts_mut()
            .clear();
    });
    let opened = spookery_engine::World::open(&pack).unwrap_or_else(|err| panic!("open: {err}"));
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
    let fired = list_round_keys(pack).iter().any(|key| {
        support::read_round(pack, key)["extra_steps"]
            .as_array()
            .is_some_and(|steps| {
                steps
                    .iter()
                    .any(|step| step["rule"] == "cast:mira/leave_door_note")
            })
    });
    assert!(fired, "leave_door_note did not run");
    let now = support::state_of(pack);
    assert!(
        now.notes
            .iter()
            .any(|message| message.id == "note_mira_door"),
        "the door note was not stored"
    );
}

fn play(world: &mut World, actor: &str, line: &str) {
    let pack = world.pack();
    let lock = spookery_engine::lock_pack(pack).unwrap();
    let mut opened =
        spookery_engine::World::resume(pack, lock).unwrap_or_else(|err| panic!("resume: {err}"));
    let seated = Presence {
        players: 1,
        actors_present: vec![actor.to_string()],
    };
    let (verb, target) = support::split_move(line);
    let wall = opened.state().last_round_at.clone();
    opened
        .queue_player_move(&seated, actor, verb, target)
        .unwrap_or_else(|err| panic!("{line}: {}", err.token()));
    opened
        .commit_then_fast_forward(&seated, &wall)
        .unwrap_or_else(|err| panic!("{line}: {err}"));
}

fn strip_room_facts(pack: &Path) {
    let path = pack.join("world/places/mira_rooms.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read rooms: {err}"));
    let unix = text.replace("\r\n", "\n");
    let from =
        "facts:\n  - id: fact_room_lamp\n    text: A lamp burns in the room.\n    hidden: false\n";
    assert!(unix.contains(from), "room facts were not found\n{unix}");
    fs::write(&path, unix.replace(from, "facts: []\n"))
        .unwrap_or_else(|err| panic!("write rooms: {err}"));
}

fn place_of(pack: &Path, actor: &str) -> String {
    support::state_of(pack)
        .actors
        .get(actor)
        .unwrap_or_else(|| panic!("{actor} missing"))
        .place()
        .to_string()
}

fn location_effects(pack: &Path, actor: &str) -> Vec<(String, String)> {
    let mut moves = Vec::new();
    for key in list_round_keys(pack) {
        for effect in support::read_round(pack, &key)["effects"]
            .as_array()
            .unwrap()
        {
            if effect["kind"] == "location" && effect["actor_id"] == actor {
                moves.push((
                    effect["from"].as_str().unwrap().to_string(),
                    effect["to"].as_str().unwrap().to_string(),
                ));
            }
        }
    }
    moves
}
