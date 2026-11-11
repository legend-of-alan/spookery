use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing as support;
use spookery_engine::Presence;

use crate::World;

/// Rowan sits at the table so each round is a short seated one.
fn rowan_seated() -> Presence {
    Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    }
}

/// One round in which the GM has Mira play `verb` at `target`.
fn mira_plays(pack: &Path, verb: &str, target: Option<&str>) {
    let lock = spookery_engine::lock_pack(pack).unwrap();
    let mut world =
        spookery_engine::World::resume(pack, lock).unwrap_or_else(|err| panic!("open: {err}"));
    let intent = spookery_protocol::QueueArgs {
        id: Some(format!("mira-{}", world.state().round_index + 1)),
        round_index: None,
        actor_id: Some("mira".to_string()),
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
    world
        .play(&rowan_seated(), &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
}

fn miras_place(pack: &Path) -> String {
    let saved = spookery_engine::World::peek(pack).unwrap();
    saved.state().actors().place["mira"].clone()
}

/// The exit names Mira's map lists for the Docks Shop.
fn shop_exits_on_miras_map(pack: &Path) -> Vec<String> {
    let saved = spookery_engine::World::peek(pack).unwrap();
    let places = saved.map(&"mira".into()).unwrap();
    let shop = places
        .iter()
        .find(|place| place.name == "Docks Shop")
        .unwrap_or_else(|| panic!("no shop on the map: {places:?}"));
    shop.exits.iter().map(|exit| exit.name.clone()).collect()
}

/// The trapdoor under the shop's rug, and a `dig` verb on the shop and the street that
/// teaches whoever digs `fact_trapdoor`. Mira starts in the shop and keeps no habits.
#[given(expr = "the shop has a hidden exit revealed by fact_trapdoor")]
fn the_shop_has_a_trapdoor(world: &mut World) {
    let pack = support::copy_pack("oneshot-docks", "map-last-seen");
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text
        .replace(
            "    text: Oil is on the shelf.\n",
            "    text: Oil is on the shelf.\n  - id: fact_trapdoor\n    text: A trapdoor is under the rug.\n    hidden: true\n",
        )
        .replace(
            "    locked_text: The stair door is locked.\n",
            "    locked_text: The stair door is locked.\n  - to: cellar\n    name: trapdoor\n    hidden: true\n    revealed_by: fact_trapdoor\n",
        );
    assert_ne!(text, next);
    fs::write(&path, next).unwrap();
    fs::write(
        pack.join("world/places/cellar.md"),
        "---\nschema: place\nid: cellar\ndisplay_name: Cellar\nfacts: []\nexits: []\n---\nUnder the shop.\n",
    )
    .unwrap();
    let dig = "  - id: dig
    when: { verb: dig }
    do:
      - reveal: { to: self, fact: fact_trapdoor }";
    for place in ["docks_shop", "docks_street"] {
        let path = pack.join(format!("world/places/{place}.md"));
        let text = fs::read_to_string(&path).unwrap();
        let next = text.replacen("exits:", &format!("rules:\n{dig}\nexits:"), 1);
        assert_ne!(next, text, "{place} has no exits");
        fs::write(&path, next).unwrap();
    }
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        text.replace("  eligible: true\n", "  eligible: false\n"),
    )
    .unwrap();
    support::edit_save(&pack, |state| {
        state
            .actors_mut()
            .set_place(&"mira".into(), "docks_shop")
            .unwrap();
    });
    world.pack = Some(pack);
}

#[given(expr = "Mira has visited the shop and left for the street")]
fn mira_has_visited_the_shop_and_left(world: &mut World) {
    mira_plays(world.pack(), "wait", None);
    mira_plays(world.pack(), "travel", Some("docks_street"));
    assert_eq!(miras_place(world.pack()), "docks_street");
}

#[given(expr = "Mira is standing in the shop")]
fn mira_is_standing_in_the_shop(world: &mut World) {
    mira_plays(world.pack(), "wait", None);
    assert!(!shop_exits_on_miras_map(world.pack()).contains(&"trapdoor".to_string()));
}

#[when(expr = "Mira learns fact_trapdoor on the street")]
fn mira_learns_on_the_street(world: &mut World) {
    mira_plays(world.pack(), "dig", None);
    let saved = spookery_engine::World::peek(world.pack()).unwrap();
    let actors = &saved.state().actors();
    assert_eq!(actors.place["mira"], "docks_street");
    assert!(actors.known_facts["mira"].contains(&"fact_trapdoor".to_string()));
}

#[when(expr = "Mira learns fact_trapdoor")]
fn mira_learns(world: &mut World) {
    mira_plays(world.pack(), "dig", None);
    assert_eq!(miras_place(world.pack()), "docks_shop");
}

#[when(expr = "Mira walks back to the shop")]
fn mira_walks_back(world: &mut World) {
    mira_plays(world.pack(), "travel", Some("docks_shop"));
    assert_eq!(miras_place(world.pack()), "docks_shop");
}

#[then(expr = "Mira's map shows the shop without the trapdoor")]
fn the_map_has_no_trapdoor(world: &mut World) {
    let exits = shop_exits_on_miras_map(world.pack());
    assert_eq!(exits, ["docks_street", "stairs"]);
}

#[then(expr = "Mira's map shows the shop with the trapdoor")]
fn the_map_has_the_trapdoor(world: &mut World) {
    let exits = shop_exits_on_miras_map(world.pack());
    assert_eq!(exits, ["docks_street", "stairs", "trapdoor"]);
}

#[then(expr = "Mira's map shows the shop with the trapdoor after that round")]
fn the_map_has_the_trapdoor_after_that_round(world: &mut World) {
    the_map_has_the_trapdoor(world);
}
