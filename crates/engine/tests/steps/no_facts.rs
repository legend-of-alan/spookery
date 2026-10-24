//! A place with no facts still accepts moves and habits.

use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::model::{Now, Presence};
use spookery_engine::store::{emit_save, load_save};
use spookery_engine::testing::journal::list_tick_keys;

use crate::support;
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
    let moves = location_moves(pack(world), "pc_lark");
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
    assert_eq!(place_of(pack(world), "pc_lark"), "gate_yard");
}

#[when(expr = "Nell waits")]
fn nell_waits(world: &mut World) {
    play(world, "pc_nell", "wait");
}

#[then(expr = "the wait commits with nothing rejected")]
fn the_wait_commits_with_nothing_rejected(world: &mut World) {
    let pack = pack(world);
    let mut found = false;
    for key in list_tick_keys(pack) {
        let round = read_round(pack, &key);
        let waited = round["intents"].as_array().is_some_and(|intents| {
            intents
                .iter()
                .any(|intent| intent["move"] == "wait" && intent["actor_id"] == "pc_nell")
        });
        if !waited {
            continue;
        }
        found = true;
        let rejected = round["rejected"].as_array().unwrap();
        assert!(rejected.is_empty(), "{key}: {rejected:?}");
    }
    assert!(found, "Nell's wait was not committed");
}

#[given(expr = "Mira stands in a place with no facts")]
fn mira_stands_in_a_place_with_no_facts(world: &mut World) {
    let pack = support::temp_pack("no-facts");
    strip_room_facts(&pack);
    edit_save(&pack, |now| {
        now.actors
            .get_mut("mira")
            .unwrap()
            .known_facts_mut()
            .clear();
    });
    let opened = spookery_engine::World::open(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    let view = opened
        .perceive("mira")
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
    let pack = pack(world);
    spookery_engine::play_pack_once(pack).unwrap_or_else(|err| panic!("round: {err}"));
}

#[then(expr = "Mira's habit fires")]
fn miras_habit_fires(world: &mut World) {
    let pack = pack(world);
    let fired = list_tick_keys(pack).iter().any(|key| {
        read_round(pack, key)["steps_run"]
            .as_array()
            .is_some_and(|steps| steps.iter().any(|step| step == "cast:mira/leave_door_note"))
    });
    assert!(fired, "leave_door_note did not run");
    let now = now_of(pack);
    assert!(
        now.messages
            .iter()
            .any(|message| message.id == "note_mira_door"),
        "the door note was not stored"
    );
}

fn pack(world: &World) -> &Path {
    world.pack.as_deref().unwrap()
}

fn now_of(pack: &Path) -> Now {
    let report = spookery_engine::testing::from_journal(pack, None, None).unwrap();
    load_save(std::str::from_utf8(&report.now_yaml).unwrap()).unwrap()
}

fn edit_save(pack: &Path, edit: impl FnOnce(&mut Now)) {
    let path = pack.join("state/save.yaml");
    let mut now = load_save(&fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut now);
    now.pack_rev = spookery_engine::canon::pack_rev(pack).unwrap();
    fs::write(&path, emit_save(&now).unwrap()).unwrap();
}

fn play(world: &mut World, actor: &str, line: &str) {
    let pack = pack(world);
    let mut opened =
        spookery_engine::World::resume(pack).unwrap_or_else(|err| panic!("resume: {err}"));
    opened.set_presence(Presence {
        humans: 1,
        actors_present: vec![actor.to_string()],
    });
    let (verb, target) = line
        .split_once(' ')
        .map_or((line, None), |(v, t)| (v, Some(t)));
    spookery_engine::handle_player_move(&mut opened, actor, verb, target, false)
        .unwrap_or_else(|err| panic!("{line}: {}", err.token()));
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
    now_of(pack)
        .actors
        .get(actor)
        .unwrap_or_else(|| panic!("{actor} missing"))
        .place()
        .to_string()
}

fn location_moves(pack: &Path, actor: &str) -> Vec<(String, String)> {
    let mut moves = Vec::new();
    for key in list_tick_keys(pack) {
        for effect in read_round(pack, &key)["effects"].as_array().unwrap() {
            if effect["class"] == "location" && effect["actor_id"] == actor {
                moves.push((
                    effect["from"].as_str().unwrap().to_string(),
                    effect["to"].as_str().unwrap().to_string(),
                ));
            }
        }
    }
    moves
}

fn read_round(pack: &Path, key: &str) -> Value {
    let text = spookery_engine::testing::journal::read_record_line(pack, key)
        .unwrap_or_else(|err| panic!("read {key}: {}", err.detail));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{key}: {err}"))
}
