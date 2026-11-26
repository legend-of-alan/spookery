//! A docks round plays a PC's verbs.

use cucumber::{then, when};
use spookery_engine::testing as support;
use spookery_engine::Presence;

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
        .queue_player_verb(&seated, &actor.into(), verb, target)
        .unwrap_or_else(|err| panic!("{line}: {}", err.code.as_str()));
    let outputs = opened
        .commit_then_fast_forward(&seated, &wall)
        .unwrap_or_else(|err| panic!("{line}: {err}"));
    let reports = outputs.iter().flat_map(|output| &output.reports);
    world.scratch = super::round_play::list_refusals(reports);
}
