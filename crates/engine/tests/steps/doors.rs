use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing as support;
use spookery_engine::Presence;

use crate::World;

/// One round in which the GM has `actor` play `verb` at `target`. Rowan sits at the table, so
/// the round is a short seated one. Returns the round's journal line.
fn plays(pack: &Path, actor: &str, verb: &str, target: Option<&str>) -> serde_json::Value {
    let lock = spookery_engine::lock_pack(pack).unwrap();
    let mut world =
        spookery_engine::World::resume(pack, lock).unwrap_or_else(|err| panic!("open: {err}"));
    let round = world.state().round_index + 1;
    let intent = spookery_protocol::IntentArgs {
        id: Some(format!("{actor}-{round}")),
        round_index: None,
        actor_id: Some(actor.to_string()),
        move_name: Some(verb.to_string()),
        target: target.map(str::to_string),
        message_to: None,
        rationale_code: "test".to_string(),
    };
    world
        .queue_gm_intent(&spookery_engine::Presence::default(), intent, || {
            panic!("the GM chose the id")
        })
        .unwrap_or_else(|code| panic!("queue: {code}"));
    let wall = world.state().last_round_at.clone();
    let seated = Presence {
        players: 1,
        actors_present: vec!["pc_rowan".to_string()],
    };
    world
        .play(&seated, &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
    drop(world);
    support::read_round(pack, &format!("round-{round:04}"))
}

fn place_of(pack: &Path, actor: &str) -> String {
    let saved = spookery_engine::World::peek(pack).unwrap();
    saved.state().actors[actor].place.clone()
}

fn stairs_open(pack: &Path) -> bool {
    let saved = spookery_engine::World::peek(pack).unwrap();
    saved
        .state()
        .places
        .get("docks_shop")
        .is_some_and(|shop| shop.open.contains(&"stairs".to_string()))
}

/// The docks pack with Mira on the street holding brass_key and keeping no habits, a `bolt`
/// verb in the shop that locks the stair door again, and a change rule that marks the world
/// `creaked` when a door opens.
#[given(expr = "the stair door in the shop needs brass_key")]
fn the_stair_door_needs_the_key(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "door");
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap();
    let bolt = "rules:
  - id: bolt
    when: { verb: bolt }
    do:
      - flag: { who: \"exit:docks_shop.stairs\", key: open, value: false }
  - id: creak
    when: { change: { flag: open, becomes: true } }
    do:
      - flag: { who: world, key: creaked, value: true }
exits:";
    let next = text.replacen("exits:", bolt, 1);
    assert_ne!(next, text);
    fs::write(&path, next).unwrap();
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        text.replace("  eligible: true\n", "  eligible: false\n"),
    )
    .unwrap();
    support::edit_save(&pack, |state| {
        let mira = state.actors.get_mut("mira").unwrap().frontier_mut();
        mira.place = "docks_street".to_string();
        mira.inventory.insert("brass_key".to_string(), 1);
    });
    world.pack = Some(pack);
}

#[when(expr = "Mira arrives at the shop holding brass_key")]
fn mira_arrives_with_the_key(world: &mut World) {
    assert!(!stairs_open(world.pack()));
    plays(world.pack(), "mira", "travel", Some("docks_shop"));
    assert_eq!(place_of(world.pack(), "mira"), "docks_shop");
}

#[given(expr = "Mira opened the stair door")]
fn mira_opened_the_door(world: &mut World) {
    the_stair_door_needs_the_key(world);
    mira_arrives_with_the_key(world);
    plays(world.pack(), "mira", "travel", Some("docks_street"));
    assert_eq!(place_of(world.pack(), "mira"), "docks_street");
    assert!(stairs_open(world.pack()));
}

#[then(expr = "after that round the stair door is open")]
fn the_door_is_open(world: &mut World) {
    assert!(stairs_open(world.pack()));
}

#[when(expr = "Mira drops brass_key")]
fn mira_drops_the_key(world: &mut World) {
    plays(world.pack(), "mira", "drop", Some("brass_key"));
    let saved = spookery_engine::World::peek(world.pack()).unwrap();
    assert_eq!(saved.state().actors["mira"].item_count("brass_key"), 0);
}

#[then(expr = "Mira can still take the stairs")]
fn mira_takes_the_stairs(world: &mut World) {
    let round = plays(world.pack(), "mira", "travel", Some("stairs"));
    assert_eq!(place_of(world.pack(), "mira"), "mira_rooms", "{round}");
}

#[when(expr = "Aldric, who has no key, takes the stairs")]
fn aldric_takes_the_stairs(world: &mut World) {
    let saved = spookery_engine::World::peek(world.pack()).unwrap();
    assert_eq!(saved.state().actors["aldric"].item_count("brass_key"), 0);
    drop(saved);
    world.scratch = plays(world.pack(), "aldric", "travel", Some("stairs")).to_string();
}

#[then(expr = "Aldric reaches Mira's rooms")]
fn aldric_reaches_the_rooms(world: &mut World) {
    assert_eq!(
        place_of(world.pack(), "aldric"),
        "mira_rooms",
        "{}",
        world.scratch
    );
}

#[when(expr = "a rule runs a flag effect on exit:docks_shop.stairs setting open to false")]
fn a_rule_locks_the_door(world: &mut World) {
    let round = plays(world.pack(), "aldric", "bolt", None);
    assert!(round["refused"].as_array().unwrap().is_empty(), "{round}");
    assert!(!stairs_open(world.pack()));
}

#[then(expr = "the GM's travel for Aldric up the stairs is refused NO_EXIT at once")]
fn the_door_refuses_aldric(world: &mut World) {
    let lock = spookery_engine::lock_pack(world.pack()).unwrap();
    let mut opened = spookery_engine::World::resume(world.pack(), lock).unwrap();
    let intent = spookery_protocol::IntentArgs {
        id: Some("aldric-stairs".to_string()),
        round_index: None,
        actor_id: Some("aldric".to_string()),
        move_name: Some("travel".to_string()),
        target: Some("stairs".to_string()),
        message_to: None,
        rationale_code: "test".to_string(),
    };
    let queued = opened.queue_gm_intent(&Presence::default(), intent, || {
        panic!("the GM chose the id")
    });
    assert_eq!(queued.unwrap_err(), spookery_engine::Reason::NoExit);
    assert!(opened.queued_intents().is_empty());
    drop(opened);
    assert_eq!(place_of(world.pack(), "aldric"), "docks_shop");
}

#[then(expr = "it opens again only when someone holding brass_key is in the shop")]
fn it_opens_again_with_the_key(world: &mut World) {
    plays(world.pack(), "aldric", "wait", None);
    assert!(!stairs_open(world.pack()), "a round with no key opened it");
    plays(world.pack(), "mira", "travel", Some("docks_shop"));
    assert!(stairs_open(world.pack()));
}

#[then(expr = "the journal has a flag effect setting open on exit:docks_shop.stairs to true")]
fn the_journal_has_the_open(world: &mut World) {
    let round = support::read_round(world.pack(), "round-0001");
    let opened = round["effects"].as_array().unwrap().iter().any(|effect| {
        effect["kind"] == "flag"
            && effect["holder"] == serde_json::json!({ "exit": ["docks_shop", "stairs"] })
            && effect["key"] == "open"
            && effect["value"] == true
    });
    assert!(opened, "{round}");
}

#[then(expr = "a change rule watching open has run in that round")]
fn the_watch_ran(world: &mut World) {
    let saved = spookery_engine::World::peek(world.pack()).unwrap();
    assert!(saved.state().flags.contains(&"creaked".to_string()));
    drop(saved);
    let round = support::read_round(world.pack(), "round-0001");
    let creaked = round["effects"].as_array().unwrap().iter().any(|effect| {
        effect["kind"] == "flag" && effect["holder"] == "world" && effect["key"] == "creaked"
    });
    assert!(creaked, "{round}");
}

#[then(expr = "spook verify replays the journal and passes")]
fn verify_passes(world: &mut World) {
    spookery_engine::verify_pack(world.pack(), None).unwrap_or_else(|err| panic!("verify: {err}"));
}
