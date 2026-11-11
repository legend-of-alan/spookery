//! Steps for one action lifecycle: start, pending, finish and cooldown, on the docks pack.

use std::fs;

use cucumber::{given, then, when};
use spookery_engine::{Presence, World as Engine};

use super::common::give_key;
use super::rules::open;
use crate::support::{append, copy_docks, edit_save, read_round, stamp_pack_rev, state_of};
use crate::World;

fn seated() -> Presence {
    Presence {
        players: 2,
        actors_present: vec!["pc_rowan".into(), "pc_wren".into()],
    }
}

fn play(opened: &mut Engine) {
    let wall = opened.state().last_round_at.clone();
    opened
        .play(&seated(), &wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
}

/// Queue Rowan's `line`, or panic with its refusal.
fn queue(opened: &mut Engine, line: &str) {
    let (verb, target) = line
        .split_once(' ')
        .map_or((line, None), |(v, t)| (v, Some(t)));
    if let Err(refusal) = opened.queue_player_move(&seated(), &"pc_rowan".into(), verb, target) {
        panic!("{line}: {}", refusal.code.as_str());
    }
}

fn add_verb(world: &World, rule: &str) {
    let pack = world.pack.as_ref().unwrap();
    append(&pack.join("rules/verbs.yaml"), rule);
    stamp_pack_rev(pack);
}

#[given(expr = "a docks table with Rowan and Wren seated in the shop")]
fn rowan_and_wren_in_the_shop(world: &mut World) {
    let pack = copy_docks("lifecycle");
    edit_save(&pack, |now| {
        now.actors_mut()
            .set_place(&"pc_wren".into(), "docks_shop")
            .unwrap();
    });
    world.pack = Some(pack);
}

#[given(expr = "the stairs take {int} ticks and Rowan holds the brass key")]
fn the_stairs_take(world: &mut World, ticks: u64) {
    let pack = world.pack.clone().unwrap();
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace(
        "    name: stairs\n    cost:\n      ticks: 2\n",
        &format!("    name: stairs\n    cost:\n      ticks: {ticks}\n"),
    );
    assert_ne!(next, text, "the stairs cost moved");
    fs::write(&path, next).unwrap();
    edit_save(&pack, give_key);
}

#[given(expr = "the rule {string} teleports Rowan to mira_rooms and takes {int} ticks")]
fn a_timed_teleport(world: &mut World, verb: String, ticks: u64) {
    add_verb(
        world,
        &format!(
            "\n  - id: {verb}\n    when:\n      verb: {verb}\n    cost:\n      ticks: {ticks}\n    do:\n      - location:\n          to: mira_rooms\n          teleport: true\n"
        ),
    );
}

#[given(expr = "the rule {string} takes {int} ticks and has a cooldown of {int} ticks")]
fn a_cooldown(world: &mut World, verb: String, ticks: u64, cooldown: u64) {
    add_verb(
        world,
        &format!(
            "\n  - id: {verb}\n    when:\n      verb: {verb}\n    cost:\n      ticks: {ticks}\n    cooldown: {cooldown}\n    do: []\n"
        ),
    );
}

#[given(expr = "the rule {string} does not count its start as an act")]
fn a_free_start(world: &mut World, verb: String) {
    add_verb(
        world,
        &format!(
            "\n  - id: {verb}\n    when:\n      verb: {verb}\n    start:\n      counts_as_act: false\n    do: []\n"
        ),
    );
}

/// A timed rule whose finish sets the world flag `<verb>_done`.
fn timed_verb(verb: &str, ticks: u64, extra: &str) -> String {
    format!(
        "
  - id: {verb}
    when:
      verb: {verb}
    cost:
      ticks: {ticks}
{extra}    do:
      - flag:
          who: world
          key: {verb}_done
          value: true
"
    )
}

#[given(expr = "the rule {string} takes {int} ticks")]
fn a_timed_rule(world: &mut World, verb: String, ticks: u64) {
    add_verb(world, &timed_verb(&verb, ticks, ""));
}

#[given(expr = "the rule {string} takes {int} ticks, is cancellable and costs {int} rest")]
fn a_cancellable_rule(world: &mut World, verb: String, ticks: u64, rest: u64) {
    let extra = format!(
        "      meters:
        rest: {rest}
    cancellable: true
"
    );
    add_verb(world, &timed_verb(&verb, ticks, &extra));
}

#[given(expr = "the rule \"travel\" is cancellable")]
fn travel_is_cancellable(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let path = pack.join("rules/verbs.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace(
        "      target: exit
    do:
      - location:
          via: target
",
        "      target: exit
    cancellable: true
    do:
      - location:
          via: target
",
    );
    assert_ne!(next, text, "the travel rule moved");
    fs::write(&path, next).unwrap();
    stamp_pack_rev(pack);
}

#[then(expr = "{string} never finished and Rowan paid {int} rest")]
fn never_finished(world: &mut World, verb: String, rest: i64) {
    let now = state_of(world.pack.as_ref().unwrap());
    assert!(
        !now.globals().flags.contains(&format!("{verb}_done")),
        "{:?}",
        now.globals().flags
    );
    let act = &now.actors().acts["pc_rowan"];
    assert_eq!(act.pending, None);
    assert_eq!(act.busy_until, None);
    let meter = &now.actors().meters[&("pc_rowan".into(), "rest".to_string())];
    assert_eq!(meter.base + meter.offset, 3 - rest);
}

#[then(expr = "{string} finished")]
fn finished(world: &mut World, verb: String) {
    let now = state_of(world.pack.as_ref().unwrap());
    assert!(
        now.globals().flags.contains(&format!("{verb}_done")),
        "{:?}",
        now.globals().flags
    );
}

#[then(expr = "spook check refuses the verb {string}")]
fn spook_check_refuses_the_verb(world: &mut World, verb: String) {
    let diags = spookery_engine::check_pack(world.pack.as_ref().unwrap());
    let refused = diags.iter().any(|diag| {
        diag.message
            .contains(&format!("verb '{verb}' is a built-in command"))
    });
    assert!(refused, "{diags:?}");
}

#[given(expr = "Rowan stands on {word} in the save")]
fn rowan_stands_on(world: &mut World, place: String) {
    edit_save(world.pack.as_ref().unwrap(), |now| {
        now.actors_mut()
            .set_place(&"pc_rowan".into(), &place)
            .unwrap();
    });
}

#[then(expr = "spook check refuses Rowan's place")]
fn spook_check_refuses_rowans_place(world: &mut World) {
    let diags = spookery_engine::check_pack(world.pack.as_ref().unwrap());
    let refused = diags
        .iter()
        .any(|diag| diag.path.as_deref() == Some("actors.place.pc_rowan"));
    assert!(refused, "{diags:?}");
}

#[when(expr = "Rowan plays {string}")]
fn rowan_plays(world: &mut World, line: String) {
    let mut opened = open(world.pack.as_ref().unwrap());
    queue(&mut opened, &line);
    play(&mut opened);
}

#[when(expr = "Rowan plays {string} and {string} in the same round")]
fn rowan_plays_both(world: &mut World, first: String, second: String) {
    let mut opened = open(world.pack.as_ref().unwrap());
    queue(&mut opened, &first);
    queue(&mut opened, &second);
    play(&mut opened);
}

#[when(expr = "rounds pass until tick {int}")]
fn rounds_pass_until(world: &mut World, tick: u64) {
    let mut opened = open(world.pack.as_ref().unwrap());
    while opened.state().world_time < tick {
        play(&mut opened);
    }
    assert_eq!(opened.state().world_time, tick);
}

#[when(expr = "the GM queues Mira's {string} outside the hours its if allows")]
fn the_gm_queues_miras(world: &mut World, habit: String) {
    let mut opened = open(world.pack.as_ref().unwrap());
    let spec = spookery_protocol::IntentArgs {
        id: Some("gm-1".to_string()),
        round_index: None,
        actor_id: Some("mira".to_string()),
        move_name: Some(habit),
        target: None,
        message_to: None,
        rationale_code: String::new(),
    };
    world.scratch = match opened.queue_gm_intent(&seated(), spec, || Ok("gm-1".into())) {
        Ok(_) => "queued".to_string(),
        Err(code) => code.as_str().to_string(),
    };
}

#[then(expr = "Rowan is at {word}")]
fn rowan_is_at(world: &mut World, place: String) {
    let now = state_of(world.pack.as_ref().unwrap());
    assert_eq!(now.actors().place["pc_rowan"], place);
}

#[then(expr = "Rowan holds lamp_oil")]
fn rowan_holds_lamp_oil(world: &mut World) {
    let now = state_of(world.pack.as_ref().unwrap());
    let oil = now
        .actors()
        .items
        .get(&("pc_rowan".into(), "lamp_oil".to_string()));
    assert_eq!(oil.copied().unwrap_or(0), 1);
}

#[then(expr = "Rowan has used no act")]
fn rowan_used_no_act(world: &mut World) {
    let now = state_of(world.pack.as_ref().unwrap());
    assert_eq!(now.actors().acts["pc_rowan"].last_act_round, 0);
}

#[then(expr = "Rowan has used the act of round {int}")]
fn rowan_used_the_act(world: &mut World, round: u64) {
    let now = state_of(world.pack.as_ref().unwrap());
    assert_eq!(now.actors().acts["pc_rowan"].last_act_round, round);
}

#[then(expr = "Wren's look does not show Rowan")]
fn wrens_look_does_not_show_rowan(world: &mut World) {
    let opened = open(world.pack.as_ref().unwrap());
    let view = opened.perceive(&seated(), "pc_wren").unwrap();
    assert!(
        !view.visible_actors.iter().any(|id| id == "pc_rowan"),
        "{:?}",
        view.visible_actors
    );
}

#[then(expr = "Rowan's look shows {string}")]
fn rowans_look_shows(world: &mut World, line: String) {
    let opened = open(world.pack.as_ref().unwrap());
    let view = opened.perceive(&seated(), "pc_rowan").unwrap();
    assert_eq!(view.busy, line);
}

#[then(expr = "Rowan's {string} is refused {word} at once")]
fn refused_at_once(world: &mut World, line: String, code: String) {
    let mut opened = open(world.pack.as_ref().unwrap());
    let refusal = opened
        .queue_player_move(&seated(), &"pc_rowan".into(), &line, None)
        .expect_err("the line is refused");
    assert_eq!(refusal.code.as_str(), code);
    assert!(opened.queued_intents().is_empty());
}

#[then(expr = "Rowan's {string} lands")]
fn rowans_line_lands(world: &mut World, line: String) {
    let mut opened = open(world.pack.as_ref().unwrap());
    queue(&mut opened, &line);
    let round = opened.state().round_index + 1;
    play(&mut opened);
    let record = read_round(world.pack.as_ref().unwrap(), &format!("round-{round:04}"));
    assert_eq!(record["refused"], serde_json::json!([]), "{record}");
}

#[then(expr = "that round refused nothing")]
fn that_round_refused_nothing(world: &mut World) {
    let record = read_round(world.pack.as_ref().unwrap(), "round-0001");
    assert_eq!(record["refused"], serde_json::json!([]), "{record}");
}

#[then(expr = "the GM is refused at once")]
fn the_gm_is_refused_at_once(world: &mut World) {
    assert_ne!(world.scratch, "queued");
}

#[then(expr = "a round later the shop still holds {int} lamp_oil")]
fn a_round_later_the_shop_holds(world: &mut World, count: u32) {
    let mut opened = open(world.pack.as_ref().unwrap());
    play(&mut opened);
    let oil = ("docks_shop".into(), "lamp_oil".to_string());
    assert_eq!(
        opened.state().places().items.get(&oil).copied(),
        Some(count)
    );
}
