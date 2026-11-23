use spookery_engine::component;
use std::fs;

use cucumber::{given, then, when};

use super::{common, table};
use crate::World;

#[given(expr = "rowan starts in docks_street")]
fn rowan_starts_in_docks_street(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "arrive");
    crate::support::edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_street")
            .unwrap();
    });
    world.pack = Some(pack);
}

/// The latch is tagged a weapon so Rowan may wield it. The street's exits stay hidden behind
/// the ledger fact, which Rowan does not know.
#[given(
    expr = "a docks copy where rowan carries two tonics and wields the latch, the world shows dawn and the street's exits are hidden"
)]
fn rowan_wields_the_latch_and_the_world_shows_dawn(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "view-sections");
    let weapon = "tags:\n  - weapon\n";
    common::replace_in_file(&pack.join("world/items/latch.md"), "tags: []\n", weapon);
    let dawn = "schema: meters\nmeters:\n  - id: dawn\n    min: 0\n    base_cap: 6\n    max: 6\n    base: 6\n    hidden: false\n";
    fs::write(pack.join("world/meters.yaml"), dawn).unwrap();
    let hidden = "    hidden: true\n    revealed_by: fact_ledger_short\n";
    crate::support::split_path(&pack, ["docks_shop", "docks_street"], ["", hidden]);
    crate::support::split_path(&pack, ["docks_street", "gate_yard"], [hidden, ""]);
    crate::support::edit_save(&pack, |now| {
        let rowan = "rowan".into();
        let mut actors = now.actors_mut();
        let mut held = actors.get_mut::<component::Item>(&rowan);
        held.shift("tonic", 2, None).unwrap();
        held.shift("latch", 1, None).unwrap();
        let mut wielded = actors.get_mut::<component::WieldedItem>(&rowan);
        wielded.set(Some("latch")).unwrap();
    });
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    world.pack = Some(pack);
}

/// The view of Docks Street ends `Exits:` `none`, with no `Items here:` heading for its empty
/// floor.
#[then(expr = "the Docks Street view shows no exits and no Items here heading")]
async fn the_docks_street_view_shows_no_exits(world: &mut World) {
    let find_street_view = |stdout: &str| {
        let start = stdout.rfind("You are in Docks Street.")?;
        let exits = stdout[start..].find("Exits:\n")? + "Exits:\n".len();
        let end = exits + stdout[start + exits..].find('\n')?;
        Some(stdout[start..start + end].to_string())
    };
    common::wait_play_until(
        world,
        |stdout| find_street_view(stdout).is_some(),
        "Docks Street",
    )
    .await;
    let view = find_street_view(&world.stdout).unwrap();
    assert!(view.ends_with("Exits:\nnone"), "{view}");
    assert!(!view.contains("Items here:"), "{view}");
}

#[given(expr = "wren is seated and not travelling")]
async fn wren_is_seated(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    table::seat_actor(world, &pack, "wren").await;
}

#[then(expr = "the seated player sees {string}")]
async fn the_seated_player_sees(world: &mut World, text: String) {
    let added = if world.player_added.contains(&text) {
        world.player_added.clone()
    } else {
        table::wait_player_from(world, world.mark, &text).await
    };
    assert!(added.contains(&text), "{added}");
}

#[given(expr = "the CLI is connected as a player holding rowan")]
async fn the_cli_holds_rowan(world: &mut World) {
    world.pack = Some(crate::support::copy_pack("docks", "cli"));
    common::play_as_pc(world, "rowan", &[]).await;
}

#[when(expr = "I type {string}")]
fn i_type(world: &mut World, line: String) {
    world.mark = common::capture_play_stdout(world).len();
    common::write_play(world, &line);
}

/// `You are in` and `Here:` are the client's words for a view, not engine prose.
#[then(expr = "I see the place name, exits and who is here, built from server messages")]
async fn i_see_the_view(world: &mut World) {
    // repeats module: mirrors its step text
    let shown = [
        "You are in Docks Shop.",
        "The shop faces the water. Tins sit in a row.",
        "Here: Aldric, You",
        "Exits:\nDocks Street — 0 ticks\nstairs (locked) — 2 ticks",
    ];
    let mark = world.mark;
    common::wait_play_until(
        world,
        |stdout| shown.iter().all(|text| stdout[mark..].contains(text)),
        "the look",
    )
    .await;
}

#[then(expr = "I see the lines {string}, {string} and {string}")]
async fn i_see_the_lines(world: &mut World, first: String, second: String, third: String) {
    let mark = world.mark;
    common::wait_play_until(
        world,
        |stdout| {
            let lines: Vec<&str> = stdout[mark..].lines().collect();
            [&first, &second, &third]
                .iter()
                .all(|text| lines.contains(&text.as_str()))
        },
        "the lines",
    )
    .await;
}

/// Writes `rules` into the frontmatter of the docks page at `page`, in place of `from`, then
/// seats play as rowan.
async fn add_rule_and_join(world: &mut World, label: &str, page: &str, from: &str, rules: &str) {
    let pack = crate::support::copy_pack("docks", label);
    let path = pack.join(page);
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replacen(from, rules, 1);
    assert_ne!(text, next);
    fs::write(&path, next).unwrap();
    crate::support::stamp_pack_rev(&pack);
    world.pack = Some(pack);
    common::play_as_pc(world, "rowan", &[]).await;
}

#[given(expr = "Aldric has an ask verb and the CLI holds rowan in the shop")]
async fn aldric_has_an_ask_verb(world: &mut World) {
    let from = "  eligible: false\n";
    let rules = "  eligible: false\nrules:\n  - id: ask\n    when:\n      verb: ask\n      target: actor\n    do: []\n";
    add_rule_and_join(world, "help-cast", "cast/aldric.md", from, rules).await;
}

#[given(expr = "the docks have a greet verb that takes an actor, and the CLI holds rowan")]
async fn the_docks_have_a_greet_verb(world: &mut World) {
    let from = "  - id: wait
";
    let rules = "  - id: greet
    when:
      verb: greet
      target: actor
    do: []

  - id: wait
";
    add_rule_and_join(world, "help-actor", "rules/verbs.yaml", from, rules).await;
}

#[given(expr = "the shop has a pray verb if rowan holds lamp_oil, and the CLI holds rowan")]
async fn the_shop_has_a_pray_verb(world: &mut World) {
    let from = "    base: 2\n";
    let rules = "    base: 2\nrules:\n  - id: pray\n    when:\n      verb: pray\n    if:\n      - has_item:\n          subject: self\n          id: lamp_oil\n    do: []\n";
    add_rule_and_join(world, "help-if", "world/places/docks_shop.md", from, rules).await;
}

#[given(expr = "lamp_oil has a sniff verb, and the CLI holds rowan")]
async fn lamp_oil_has_a_sniff_verb(world: &mut World) {
    let rules =
        "rules:\n  - id: sniff\n    when:\n      verb: sniff\n      target: item\n    do: []\n";
    add_rule_and_join(
        world,
        "help-item",
        "world/items/lamp_oil.md",
        "rules: []\n",
        rules,
    )
    .await;
}

/// Waits for the help reply after the mark, then checks a line it must not hold.
#[then(expr = "help shows {string} and not {string}")]
async fn help_shows_and_not(world: &mut World, shown: String, hidden: String) {
    let mark = world.mark;
    common::wait_play_until(
        world,
        |stdout| stdout[mark..].lines().any(|line| line == shown),
        "the help",
    )
    .await;
    let help = &world.stdout[mark..];
    assert!(help.lines().all(|line| line != hidden), "{help}");
}
