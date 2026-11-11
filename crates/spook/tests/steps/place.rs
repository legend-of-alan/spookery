use std::fs;

use cucumber::{given, then, when};
use spookery_engine::{emit_save, parse_save};

use super::{common, table};
use crate::World;

#[given(expr = "rowan starts in docks_street")]
fn rowan_starts_in_docks_street(world: &mut World) {
    let pack = crate::support::copy_docks("arrive");
    let path = pack.join("state/save.yaml");
    let mut now = parse_save(&fs::read_to_string(&path).unwrap()).unwrap();
    now.actors_mut()
        .set_place(&"pc_rowan".into(), "docks_street")
        .unwrap();
    fs::write(&path, emit_save(&now).unwrap()).unwrap();
    world.pack = Some(pack);
}

#[given(expr = "pc_wren is seated and not travelling")]
fn pc_wren_is_seated(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    table::seat_actor(world, &pack, "pc_wren");
}

#[then(expr = "the seated player sees {string}")]
fn the_seated_player_sees(world: &mut World, text: String) {
    let added = if world.player_added.contains(&text) {
        world.player_added.clone()
    } else {
        table::wait_player_from(world, world.mark, &text)
    };
    assert!(added.contains(&text), "{added}");
}

#[given(expr = "the CLI is connected as a player holding rowan")]
fn the_cli_holds_rowan(world: &mut World) {
    world.pack = Some(crate::support::copy_docks("cli"));
    common::join_play(world, false);
}

#[when(expr = "I type {string}")]
fn i_type(world: &mut World, line: String) {
    world.mark = common::capture_play_stdout(world).len();
    common::write_play(world, &line);
}

/// `You are in` and `Here:` are the client's words for a view, not engine prose.
#[then(expr = "I see the place name, exits and who is here, built from server messages")]
fn i_see_the_place(world: &mut World) {
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
    );
}

#[then(expr = "I see the lines {string}, {string} and {string}")]
fn i_see_the_lines(world: &mut World, first: String, second: String, third: String) {
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
    );
}

/// Writes `rules` into the frontmatter of the docks page at `page`, in place of `from`, then
/// seats play as rowan.
fn add_rule_and_join(world: &mut World, label: &str, page: &str, from: &str, rules: &str) {
    let pack = crate::support::copy_docks(label);
    let path = pack.join(page);
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replacen(from, rules, 1);
    assert_ne!(text, next);
    fs::write(&path, next).unwrap();
    crate::support::stamp_pack_rev(&pack);
    world.pack = Some(pack);
    common::join_play(world, false);
}

#[given(expr = "Aldric has an ask verb and the CLI holds rowan in the shop")]
fn aldric_has_an_ask_verb(world: &mut World) {
    let from = "  eligible: false\n";
    let rules = "  eligible: false\nrules:\n  - id: ask\n    when:\n      verb: ask\n      target: actor\n    do: []\n";
    add_rule_and_join(world, "help-cast", "cast/aldric.md", from, rules);
}

#[given(expr = "the docks have a greet verb that takes an actor, and the CLI holds rowan")]
fn the_docks_have_a_greet_verb(world: &mut World) {
    let from = "  - id: wait
";
    let rules = "  - id: greet
    when:
      verb: greet
      target: actor
    do: []

  - id: wait
";
    add_rule_and_join(world, "help-actor", "rules/verbs.yaml", from, rules);
}

#[given(expr = "the shop has a pray verb if rowan holds lamp_oil, and the CLI holds rowan")]
fn the_shop_has_a_pray_verb(world: &mut World) {
    let from = "    locked_text: The stair door is locked.\n";
    let rules = "    locked_text: The stair door is locked.\nrules:\n  - id: pray\n    when:\n      verb: pray\n    if:\n      - has_item:\n          who: self\n          id: lamp_oil\n    do: []\n";
    add_rule_and_join(world, "help-if", "world/places/docks_shop.md", from, rules);
}

#[given(expr = "lamp_oil has a sniff verb, and the CLI holds rowan")]
fn lamp_oil_has_a_sniff_verb(world: &mut World) {
    let rules =
        "rules:\n  - id: sniff\n    when:\n      verb: sniff\n      target: item\n    do: []\n";
    add_rule_and_join(
        world,
        "help-item",
        "world/items/lamp_oil.md",
        "rules: []\n",
        rules,
    );
}

/// Waits for the help reply after the mark, then checks a line it must not hold.
#[then(expr = "help shows {string} and not {string}")]
fn help_shows_and_not(world: &mut World, shown: String, hidden: String) {
    let mark = world.mark;
    common::wait_play_until(
        world,
        |stdout| stdout[mark..].lines().any(|line| line == shown),
        "the help",
    );
    let help = &world.stdout[mark..];
    assert!(help.lines().all(|line| line != hidden), "{help}");
}
