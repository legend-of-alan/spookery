use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing as support;
use spookery_engine::{component, Presence};

use crate::World;

/// Rowan holds a seat, so each round is a short seated one.
fn rowan_seated() -> Presence {
    Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    }
}

/// One round in which the GM has Mira play `verb` at `target`.
fn mira_plays(pack: &Path, verb: &str, target: Option<&str>) {
    let lock = spookery_engine::lock_pack(pack).unwrap();
    let mut session =
        spookery_engine::Session::resume(pack, lock).unwrap_or_else(|err| panic!("open: {err}"));
    let intent = spookery_protocol::QueueArgs {
        id: Some(format!("mira-{}", session.world().round_index + 1)),
        round_index: None,
        actor_id: Some("mira".to_string()),
        verb_id: Some(verb.to_string()),
        target: target.map(str::to_string),
        message_to: None,
        rationale_code: "test".to_string(),
    };
    session
        .queue_gm_intent(&spookery_engine::Presence::default(), intent, || {
            panic!("the GM chose the id")
        })
        .unwrap_or_else(|code| panic!("queue: {code}"));
    let wall = session.world().last_round_at.clone();
    session
        .play(&rowan_seated(), &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
}

/// The path ids of the exits Mira's map lists for the Docks Shop.
fn shop_exits_mira_knows(pack: &Path) -> Vec<String> {
    let saved = spookery_engine::Session::peek(pack).unwrap();
    let places = saved.map(&"mira".into()).unwrap();
    let shop = places
        .iter()
        .find(|place| place.name == "Docks Shop")
        .unwrap_or_else(|| panic!("no shop on the map: {places:?}"));
    shop.exits.iter().map(|exit| exit.path_id.clone()).collect()
}

/// The trapdoor under the shop's rug, and a `dig` verb on the shop and the street that sets
/// `found_trapdoor` on whoever digs. Mira starts in the shop and keeps no habits.
#[given(expr = "the shop has an exit revealed by the found_trapdoor flag")]
fn the_shop_has_a_trapdoor(world: &mut World) {
    let pack = support::copy_pack("docks", "map-last-seen");
    support::append(
        &pack.join("world/map.yaml"),
        "  - path: docks_shop -> cellar\n    id: trapdoor\n    revealed_by: found_trapdoor\n",
    );
    fs::write(
        pack.join("world/places/cellar.md"),
        "---\nschema: place\nid: cellar\nname: Cellar\n---\nUnder the shop.\n",
    )
    .unwrap();
    let dig = "  - id: dig
    when: { verb: dig }
    do:
      - flag: { of: self, id: found_trapdoor, value: true }";
    for place in ["docks_shop", "docks_street"] {
        super::round_play::add_place_rules(&pack, place, dig);
    }
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        text.replace("  eligible: true\n", "  eligible: false\n"),
    )
    .unwrap();
    support::edit_save(&pack, |ecs_world| {
        ecs_world
            .actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
    });
    world.pack = Some(pack);
}

#[given(expr = "Mira has visited the shop and left for the street")]
fn mira_has_visited_the_shop_and_left(world: &mut World) {
    mira_plays(world.pack(), "wait", None);
    mira_plays(world.pack(), "travel", Some("docks_shop.docks_street"));
    let saved = spookery_engine::Session::peek(world.pack()).unwrap();
    assert_eq!(support::place_of(saved.world(), "mira"), "docks_street");
}

#[given(expr = "Mira is standing in the shop")]
fn mira_is_standing_in_the_shop(world: &mut World) {
    mira_plays(world.pack(), "wait", None);
    assert!(!shop_exits_mira_knows(world.pack()).contains(&"trapdoor".to_string()));
}

#[when(expr = "Mira finds the trapdoor on the street")]
fn mira_finds_the_trapdoor_on_the_street(world: &mut World) {
    mira_plays(world.pack(), "dig", None);
    let saved = spookery_engine::Session::peek(world.pack()).unwrap();
    let (ecs_world, mira) = (saved.world(), "mira".into());
    assert_eq!(support::place_of(ecs_world, "mira"), "docks_street");
    let flags = ecs_world.actors().get::<component::Flag>(&mira);
    assert!(flags.contains("found_trapdoor"));
}

#[when(expr = "Mira finds the trapdoor")]
fn mira_finds_the_trapdoor(world: &mut World) {
    mira_plays(world.pack(), "dig", None);
    let saved = spookery_engine::Session::peek(world.pack()).unwrap();
    assert_eq!(support::place_of(saved.world(), "mira"), "docks_shop");
}

#[when(expr = "Mira walks back to the shop")]
fn mira_walks_back(world: &mut World) {
    mira_plays(world.pack(), "travel", Some("docks_shop.docks_street"));
    let saved = spookery_engine::Session::peek(world.pack()).unwrap();
    assert_eq!(support::place_of(saved.world(), "mira"), "docks_shop");
}

#[then(expr = "Mira's map shows the shop without the trapdoor")]
fn the_map_has_no_trapdoor(world: &mut World) {
    let exits = shop_exits_mira_knows(world.pack());
    assert_eq!(exits, ["docks_shop.docks_street", "stairs"]);
}

#[then(expr = "Mira's map shows the shop with the trapdoor")]
fn the_map_has_the_trapdoor(world: &mut World) {
    let exits = shop_exits_mira_knows(world.pack());
    assert_eq!(exits, ["docks_shop.docks_street", "stairs", "trapdoor"]);
}

#[then(expr = "Mira's map shows the shop with the trapdoor after that round")]
fn the_map_has_the_trapdoor_after_that_round(world: &mut World) {
    the_map_has_the_trapdoor(world);
}
