use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_engine::MeterLive;
use spookery_engine::RoundCause;
use spookery_engine::{emit_save, load_save};
use spookery_engine::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_engine::{Clock, Now};

use super::steps_run::read_steps_run;
use crate::support::{self, temp_pack};
use crate::World;

#[when(expr = "nell travels via door into docks_shop")]
fn nell_travels_via_door(world: &mut World) {
    let pack = temp_pack("trigger-door");
    add_door(&pack);
    add_bell(&pack);
    add_verb(
        &pack,
        "\n  - id: blink\n    when:\n      verb: blink\n    do:\n      - location:\n          to: docks_shop\n          teleport: true\n",
    );
    flag_pcs(&pack, "heard_bell");
    write_nell(&pack, &["heard_bell"]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
    });
    play(
        world,
        &pack,
        vec![intent(
            "nell-door",
            "pc_nell",
            "travel",
            Target::Place("door".to_string()),
        )],
    );
    world.pack = Some(pack);
}

#[then(expr = "place:docks_shop\\/bell fires in the same tick")]
fn bell_fires_in_the_same_tick(world: &mut World) {
    let tick = latest(world.pack.as_ref().unwrap());
    assert!(
        read_steps_run(&tick)
            .iter()
            .any(|name| name == "place:docks_shop/bell"),
        "bell did not fire\n{tick}"
    );
}

#[then(expr = "teleporting into docks_shop by an author rule also fires it")]
fn teleport_also_fires_the_bell(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent("wren-blink", "pc_wren", "blink", Target::Empty)],
    );
    let tick = latest(&pack);
    assert!(
        read_steps_run(&tick)
            .iter()
            .any(|name| name == "place:docks_shop/bell"),
        "a teleport did not fire the bell\n{tick}"
    );
}

#[given(expr = "briar has 3 hp and a story rule on hp crosses <=0 sets flag downed")]
fn briar_has_3_hp_and_a_downed_rule(world: &mut World) {
    let pack = downed_pack(world, "trigger-downed", 3, false, "<=0");
    world.pack = Some(pack);
}

#[when(expr = "an attack removes 4 hp")]
fn an_attack_removes_4_hp(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent(
            "hit-briar",
            "pc_rowan",
            "hit",
            Target::Actor("briar".to_string()),
        )],
    );
}

#[then(expr = "briar has flag downed in the same tick")]
fn briar_has_flag_downed(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let tick = latest(pack);
    assert!(
        read_steps_run(&tick)
            .iter()
            .any(|name| name == "story:injury/downed"),
        "the downed rule did not fire\n{tick}"
    );
    assert!(
        load(pack)
            .actors
            .get("briar")
            .unwrap()
            .flags()
            .iter()
            .any(|flag| flag == "downed"),
        "briar is not downed"
    );
}

#[given(expr = "briar is already downed")]
fn briar_is_already_downed(world: &mut World) {
    let pack = downed_pack(world, "trigger-already", 1, true, "<=1");
    world.pack = Some(pack);
}

#[when(expr = "another tick passes")]
fn another_tick_passes(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent(
            "nick-briar",
            "pc_rowan",
            "nick",
            Target::Actor("briar".to_string()),
        )],
    );
}

#[then(expr = "the rule does not fire again")]
fn the_rule_does_not_fire_again(world: &mut World) {
    let tick = latest(world.pack.as_ref().unwrap());
    assert!(
        read_steps_run(&tick)
            .iter()
            .all(|name| name != "story:injury/downed"),
        "the downed rule fired again\n{tick}"
    );
}

#[given(expr = "nell has flag found_key and arrives at mira_rooms")]
fn nell_arrives_with_the_key(world: &mut World) {
    let pack = temp_pack("trigger-ledger");
    add_rooms(&pack);
    append(
        &pack.join("world/places/mira_rooms.md"),
        "    hidden: false\n",
        "    hidden: false\n  - id: fact_ledger\n    text: The ledger is in the desk.\n    hidden: false\n",
    );
    fs::create_dir_all(pack.join("story")).unwrap();
    fs::write(
        pack.join("story/smuggler.yaml"),
        "\
schema: story
id: smuggler
rules:
  - id: ledger
    when:
      change:
        arrive: mira_rooms
        who: pc
    if:
      - flag:
          who: self
          key: found_key
          is: true
    do:
      - memory:
          who: self
          fact_ids:
            - fact_ledger
      - clock:
          id: tide
          start: true
",
    )
    .unwrap();
    write_nell(&pack, &["found_key"]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
        now.actors
            .get_mut("pc_nell")
            .unwrap()
            .flags_mut()
            .push("found_key".to_string());
        now.clocks.insert(
            "tide".to_string(),
            Clock {
                value: 0,
                max: 6,
                requires_presence: false,
                owners: vec!["pc_nell".to_string()],
                paused: true,
            },
        );
    });
    play(
        world,
        &pack,
        vec![intent(
            "nell-rooms",
            "pc_nell",
            "travel",
            Target::Place("rooms".to_string()),
        )],
    );
    world.pack = Some(pack);
}

#[then(expr = "nell knows fact_ledger and clock tide is running")]
fn nell_knows_the_ledger(world: &mut World) {
    let now = load(world.pack.as_ref().unwrap());
    assert!(
        now.actors
            .get("pc_nell")
            .unwrap()
            .known_facts()
            .iter()
            .any(|fact| fact == "fact_ledger"),
        "nell does not know fact_ledger"
    );
    let tide = now.clocks.get("tide").unwrap();
    assert!(!tide.paused, "tide is paused");
}

#[given(expr = "two rules that set each other's watched flag")]
fn two_rules_set_each_others_flag(world: &mut World) {
    let pack = temp_pack("trigger-loop");
    write_nell(&pack, &["sun", "moon"]);
    add_verb(
        &pack,
        "\n  - id: raise\n    when:\n      verb: raise\n    do:\n      - flag:\n          who: self\n          key: sun\n          value: true\n",
    );
    fs::create_dir_all(pack.join("story")).unwrap();
    fs::write(
        pack.join("story/loop.yaml"),
        "\
schema: story
id: loop
rules:
  - id: sun_to_moon
    when:
      change:
        flag: sun
        becomes: true
    do:
      - flag:
          who: self
          key: moon
          value: true
      - flag:
          who: self
          key: sun
          value: false
  - id: moon_to_sun
    when:
      change:
        flag: moon
        becomes: true
    do:
      - flag:
          who: self
          key: sun
          value: true
      - flag:
          who: self
          key: moon
          value: false
",
    )
    .unwrap();
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_shop");
    });
    world.pack = Some(pack);
}

#[then(expr = "spook check warns of a cycle, and a tick stops at depth 4 and logs trigger_depth")]
fn check_warns_of_a_cycle_and_the_tick_stops(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let output = support::spook(&["check", &pack.to_string_lossy()]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "check failed\n{stdout}{stderr}");
    assert!(stderr.contains("cycle"), "{stderr}");
    assert!(stderr.contains("WARNING"), "{stderr}");
    a_tick_stops_at_depth_4(world);
}

fn a_tick_stops_at_depth_4(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent("raise-sun", "pc_nell", "raise", Target::Empty)],
    );
    let tick = latest(&pack);
    let names = read_steps_run(&tick);
    let suns = names
        .iter()
        .filter(|name| name.as_str() == "story:loop/sun_to_moon")
        .count();
    let moons = names
        .iter()
        .filter(|name| name.as_str() == "story:loop/moon_to_sun")
        .count();
    assert_eq!(suns, 2, "sun rule ran {suns} times\n{tick}");
    assert_eq!(moons, 2, "moon rule ran {moons} times\n{tick}");
    assert!(
        names.iter().any(|name| name == "trigger_depth"),
        "trigger_depth was not logged\n{tick}"
    );
}

#[given(expr = "a rule on a known fact narrates to the room")]
fn a_known_fact_narrates_to_the_room(world: &mut World) {
    let pack = knows_narrate(world, "trigger-leak-room", "here");
    world.pack = Some(pack);
}

#[given(expr = "a rule on a known fact narrates to the subject")]
fn a_known_fact_narrates_to_the_subject(world: &mut World) {
    let pack = knows_narrate(world, "trigger-leak-self", "self");
    world.pack = Some(pack);
}

fn knows_narrate(world: &mut World, label: &str, to: &str) -> std::path::PathBuf {
    let pack = temp_pack(label);
    fs::create_dir_all(pack.join("story")).unwrap();
    fs::write(
        pack.join("story/secret.yaml"),
        format!(
            "\
schema: story
id: secret
rules:
  - id: tell
    when:
      change:
        knows: fact_ledger
    do:
      - narrate:
          to: {to}
          text: A secret moved.
"
        ),
    )
    .unwrap();
    edit_save(world, &pack, |_| {});
    pack
}

#[then(expr = "spook verify passes after ticks that fired triggers")]
fn verify_passes_after_triggers(world: &mut World) {
    let pack = temp_pack("trigger-verify");
    add_door(&pack);
    add_bell(&pack);
    flag_pcs(&pack, "heard_bell");
    write_nell(&pack, &["heard_bell"]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
    });
    play(
        world,
        &pack,
        vec![intent(
            "nell-door",
            "pc_nell",
            "travel",
            Target::Place("door".to_string()),
        )],
    );
    let output = support::spook(&["verify", &pack.to_string_lossy()]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "verify failed\n{stdout}{stderr}");
    world.pack = Some(pack);
}

#[when(expr = "nell enters the bog")]
fn nell_enters_the_bog(world: &mut World) {
    let pack = temp_pack("trigger-bog");
    append(
        &pack.join("world/places/docks_street.md"),
        "    name: docks_shop\n",
        "    name: docks_shop\n  - to: bog\n    name: bog\n",
    );
    fs::write(
        pack.join("world/places/bog.md"),
        "\
---
schema: place
id: bog
display_name: Bog
facts: []
exits: []
rules:
  - id: sink
    when:
      change:
        arrive: here
        who: pc
    roll:
      check: stamina
      target: 100
    success:
      - meter:
          who: self
          key: grit
          add: 0
    failure:
      - meter:
          who: self
          key: grit
          add: -1
---
The ground gives.
",
    )
    .unwrap();
    append(
        &pack.join("rules/checks.yaml"),
        "    stat: grit\n",
        "    stat: grit\n  - id: stamina\n    dice:\n      count: 1\n      sides: 6\n    stat: grit\n",
    );
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
    });
    let (_now, rolls, tick) = play(
        world,
        &pack,
        vec![intent(
            "nell-bog",
            "pc_nell",
            "travel",
            Target::Place("bog".to_string()),
        )],
    );
    let faces = rolls.iter().any(|roll| roll.actor_id() == "pc_nell");
    assert!(faces, "nell did not roll\n{tick}");
    world.scratch = "rolled".to_string();
    world.pack = Some(pack);
}

#[then(expr = "that arrival rolled stamina and spook verify reproduces it")]
fn the_bog_roll_reproduces(world: &mut World) {
    assert_eq!(world.scratch, "rolled");
    let pack = world.pack.as_ref().unwrap();
    let now = load(pack);
    let grit = now
        .actors
        .get("pc_nell")
        .unwrap()
        .meters()
        .get("grit")
        .unwrap();
    assert_eq!(grit.offset, -1, "the miss band did not spend grit");
    let output = support::spook(&["verify", &pack.to_string_lossy()]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "verify failed\n{stdout}{stderr}");
}

#[when(expr = "nell travels into the shop and the bell narrates")]
fn nell_travels_and_the_bell_narrates(world: &mut World) {
    let pack = temp_pack("trigger-narrate");
    add_door(&pack);
    add_bell_narrate(&pack);
    flag_pcs(&pack, "heard_bell");
    write_nell(&pack, &["heard_bell"]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
    });
    let played = play_view(
        world,
        &pack,
        vec![intent(
            "nell-door",
            "pc_nell",
            "travel",
            Target::Place("door".to_string()),
        )],
    );
    world.scratch = played.tick.to_string();
    world.summary = played.summary;
    world.seen = played.seen;
    world.told = played.told;
    world.pack = Some(pack);
}

#[then(expr = "the tick narrates {string} to {string}")]
fn the_tick_narrates_to(world: &mut World, text: String, viewer: String) {
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    assert!(
        narrates(&tick, &text, &viewer),
        "missing narrate to {viewer}\n{tick}"
    );
}

#[then(expr = "the tick does not narrate {string} to {string}")]
fn the_tick_does_not_narrate_to(world: &mut World, text: String, viewer: String) {
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    assert!(
        !narrates(&tick, &text, &viewer),
        "narrate reached {viewer}\n{tick}"
    );
}

#[then(expr = "{string} is not told {string}")]
fn the_viewer_is_not_told(world: &mut World, viewer: String, text: String) {
    assert!(
        world
            .told
            .iter()
            .all(|(to, line)| to != &viewer || line != &text),
        "{viewer} was told {text:?}\n{:?}",
        world.told
    );
}

#[then(expr = "{string} is told {string}")]
fn the_viewer_is_told(world: &mut World, viewer: String, text: String) {
    assert!(
        world
            .told
            .iter()
            .any(|(to, line)| to == &viewer && line == &text),
        "not told {viewer}: {:?}\n{:?}",
        text,
        world.told
    );
}

#[given(expr = "the street has a hidden trapdoor")]
fn the_street_has_a_hidden_trapdoor(world: &mut World) {
    let pack = temp_pack("trigger-reveal");
    append(
        &pack.join("world/places/docks_street.md"),
        "    name: docks_shop\n",
        "    name: docks_shop\n  - to: cellar\n    name: trapdoor\n    hidden: true\n    revealed_by: fact_trapdoor\n",
    );
    append(
        &pack.join("world/places/docks_street.md"),
        "    hidden: false\n",
        "    hidden: false\n  - id: fact_trapdoor\n    text: A trapdoor sits under the boards.\n    hidden: true\n",
    );
    fs::write(
        pack.join("world/places/cellar.md"),
        "\
---
schema: place
id: cellar
display_name: Cellar
facts: []
exits: []
---
Under the street.
",
    )
    .unwrap();
    add_verb(
        &pack,
        "\n  - id: uncover\n    when:\n      verb: uncover\n    do:\n      - reveal:\n          who: self\n          fact: fact_trapdoor\n",
    );
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
        now.places.insert(
            "cellar".to_string(),
            spookery_engine::PlaceLive {
                items: Default::default(),
                notes: Vec::new(),
                actors: Vec::new(),
            },
        );
    });
    world.pack = Some(pack);
}

#[when(expr = "pc_nell reveals fact_trapdoor")]
fn nell_reveals_the_trapdoor(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let (now, _, _) = play(
        world,
        &pack,
        vec![intent("nell-uncover", "pc_nell", "uncover", Target::Empty)],
    );
    world.scratch = now.actors.get("pc_nell").unwrap().known_facts().join(",");
    world.pack = Some(pack);
}

#[then(expr = "pc_nell knows fact_trapdoor")]
fn nell_knows_the_trapdoor(world: &mut World) {
    assert!(
        world.scratch.split(',').any(|id| id == "fact_trapdoor"),
        "{}",
        world.scratch
    );
}

#[then(expr = "pc_nell can see the trapdoor")]
fn nell_can_see_the_trapdoor(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let world_now = spookery_engine::World::resume(pack).unwrap();
    let exits = world_now.perceive("pc_nell").unwrap().exits;
    assert!(
        exits.iter().any(|exit| exit.name == "trapdoor"),
        "trapdoor stayed hidden: {exits:?}"
    );
}

#[given(expr = "a verb sets world flag tide_turned and a rule marks whoever is present")]
fn a_verb_sets_the_tide(world: &mut World) {
    let pack = temp_pack("trigger-tide");
    add_verb(
        &pack,
        "\n  - id: turn_tide\n    when:\n      verb: turn_tide\n    do:\n      - flag:\n          who: world\n          key: tide_turned\n          value: true\n",
    );
    fs::create_dir_all(pack.join("story")).unwrap();
    fs::write(
        pack.join("story/tide.yaml"),
        "\
schema: story
id: tide
rules:
  - id: mark
    when:
      change:
        flag: tide_turned
        becomes: true
    do:
      - flag:
          who: self
          key: saw_tide
          value: true
",
    )
    .unwrap();
    write_nell(&pack, &["saw_tide"]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
    });
    world.pack = Some(pack);
}

#[when(expr = "pc_nell sets the tide")]
fn nell_sets_the_tide(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent("nell-tide", "pc_nell", "turn_tide", Target::Empty)],
    );
}

#[then(expr = "the world has flag tide_turned")]
fn the_world_has_the_tide(world: &mut World) {
    let now = load(world.pack.as_ref().unwrap());
    assert!(
        now.flags.iter().any(|flag| flag == "tide_turned"),
        "flags: {:?}",
        now.flags
    );
}

#[then(expr = "pc_nell has flag saw_tide")]
fn nell_has_saw_tide(world: &mut World) {
    let now = load(world.pack.as_ref().unwrap());
    assert!(
        now.actors
            .get("pc_nell")
            .unwrap()
            .flags()
            .iter()
            .any(|flag| flag == "saw_tide"),
        "nell flags: {:?}",
        now.actors.get("pc_nell").unwrap().flags()
    );
}

#[then(expr = "the GM summary lists {string}")]
fn the_gm_summary_lists(world: &mut World, key: String) {
    assert!(
        world.summary.iter().any(|item| item == &key),
        "missing {key}\n{:?}",
        world.summary
    );
}

#[then(expr = "{string}'s summary lists {string}")]
fn the_viewer_summary_lists(world: &mut World, viewer: String, key: String) {
    let changes = seen_of(world, &viewer);
    assert!(
        changes.iter().any(|item| item == &key),
        "{viewer} missing {key}\n{changes:?}"
    );
}

#[then(expr = "{string}'s summary does not list {string}")]
fn the_viewer_summary_omits(world: &mut World, viewer: String, key: String) {
    let changes = seen_of(world, &viewer);
    assert!(
        changes.iter().all(|item| item != &key),
        "{viewer} saw {key}\n{changes:?}"
    );
}

#[when(expr = "a round runs")]
fn a_round_runs(world: &mut World) {
    let pack = temp_pack("trigger-tick-counter");
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
    });
    show(world, pack, Vec::new());
}

#[then(expr = "the GM summary lists a tick counter change")]
fn the_gm_summary_lists_a_tick_counter(world: &mut World) {
    assert!(
        world.summary.iter().any(|item| is_tick_counter(item)),
        "no tick counter\n{:?}",
        world.summary
    );
}

#[then(expr = "{string}'s summary lists no tick counter change")]
fn the_viewer_summary_omits_tick_counters(world: &mut World, viewer: String) {
    let changes = seen_of(world, &viewer);
    assert!(
        changes.iter().all(|item| !is_tick_counter(item)),
        "{viewer} saw a tick counter\n{changes:?}"
    );
}

#[when(expr = "pc_nell learns fact_trapdoor while pc_wren is in the same room")]
fn nell_learns_the_trapdoor_beside_wren(world: &mut World) {
    let pack = temp_pack("trigger-known-fact");
    append(
        &pack.join("world/places/docks_street.md"),
        "    hidden: false\n",
        "    hidden: false\n  - id: fact_trapdoor\n    text: A trapdoor sits under the boards.\n    hidden: true\n",
    );
    add_verb(
        &pack,
        "\n  - id: learn\n    when:\n      verb: learn\n    do:\n      - reveal:\n          who: self\n          fact: fact_trapdoor\n",
    );
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
        sit(now, &["pc_nell", "pc_wren"]);
    });
    show(
        world,
        pack,
        vec![intent("nell-learn", "pc_nell", "learn", Target::Empty)],
    );
}

#[when(expr = "an item is taken from docks_shop")]
fn an_item_is_taken_from_the_shop(world: &mut World) {
    let pack = temp_pack("trigger-place-item");
    edit_save(world, &pack, |now| {
        sit(now, &["pc_rowan"]);
    });
    show(
        world,
        pack,
        vec![intent(
            "rowan-take",
            "pc_rowan",
            "take",
            Target::Actor("lamp_oil".to_string()),
        )],
    );
}

#[when(expr = "pc_rowan takes an item while pc_nell is in the same room")]
fn rowan_takes_an_item_beside_nell(world: &mut World) {
    let pack = temp_pack("trigger-actor-item");
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_shop");
        sit(now, &["pc_rowan", "pc_nell", "pc_wren"]);
    });
    show(
        world,
        pack,
        vec![intent(
            "rowan-take",
            "pc_rowan",
            "take",
            Target::Actor("lamp_oil".to_string()),
        )],
    );
}

#[when(expr = "a clock owned by pc_nell ticks")]
fn a_clock_owned_by_nell_ticks(world: &mut World) {
    let pack = temp_pack("trigger-clock");
    add_verb(
        &pack,
        "\n  - id: mark_tide\n    when:\n      verb: mark_tide\n    do:\n      - clock:\n          id: nell_tide\n          add: 1\n",
    );
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
        now.clocks.insert(
            "nell_tide".to_string(),
            Clock {
                value: 0,
                max: 6,
                requires_presence: false,
                owners: vec!["pc_nell".to_string()],
                paused: false,
            },
        );
        sit(now, &["pc_nell", "pc_wren"]);
    });
    show(
        world,
        pack,
        vec![intent("nell-tide", "pc_nell", "mark_tide", Target::Empty)],
    );
}

#[when(expr = "pc_nell travels from docks_street into docks_shop")]
fn nell_travels_from_the_street_into_the_shop(world: &mut World) {
    let pack = temp_pack("trigger-both-rooms");
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
        put(now, "pc_harbormouse", "mira_rooms");
        sit(now, &["pc_nell"]);
    });
    show(
        world,
        pack,
        vec![intent(
            "nell-shop",
            "pc_nell",
            "travel",
            Target::Place("docks_shop".to_string()),
        )],
    );
}

#[when(expr = "a rule sets world flag tide_turned")]
fn a_rule_sets_the_tide(world: &mut World) {
    let pack = temp_pack("trigger-world-flag");
    add_verb(
        &pack,
        "\n  - id: turn_tide\n    when:\n      verb: turn_tide\n    do:\n      - flag:\n          who: world\n          key: tide_turned\n          value: true\n",
    );
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_shop");
        put(now, "pc_rowan", "mira_rooms");
        sit(now, &["pc_nell", "pc_wren", "pc_rowan"]);
    });
    show(
        world,
        pack,
        vec![intent("nell-tide", "pc_nell", "turn_tide", Target::Empty)],
    );
}

#[then(expr = "every seated player's summary lists {string}")]
fn every_seated_summary_lists(world: &mut World, key: String) {
    let pack = world.pack.as_ref().unwrap();
    let now = load(pack);
    // Seats live in memory, not in the save, so read the ones `edit_save` remembered.
    let presence = remembered_presence(world, pack).unwrap_or_default();
    let seated: Vec<String> = presence
        .actors_present
        .iter()
        .filter(|id| now.actors.get(*id).is_some_and(|actor| actor.is_pc()))
        .cloned()
        .collect();
    assert!(!seated.is_empty(), "nobody is seated");
    for id in &seated {
        let changes = seen_of(world, id);
        assert!(
            changes.iter().any(|item| item == &key),
            "{id} missing {key}\n{changes:?}"
        );
    }
}

#[when(expr = "pc_wren's meter changes while pc_nell is elsewhere")]
fn wrens_meter_changes_while_nell_is_elsewhere(world: &mut World) {
    let pack = temp_pack("trigger-meter");
    add_verb(
        &pack,
        "\n  - id: spend\n    when:\n      verb: spend\n    do:\n      - meter:\n          who: self\n          key: grit\n          add: -1\n",
    );
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_shop");
        sit(now, &["pc_nell", "pc_wren", "pc_harbormouse"]);
    });
    show(
        world,
        pack,
        vec![intent("wren-spend", "pc_wren", "spend", Target::Empty)],
    );
}

#[when(expr = "a message hidden from pc_wren is written")]
fn a_message_hidden_from_wren_is_written(world: &mut World) {
    let pack = temp_pack("trigger-message");
    add_verb(
        &pack,
        "\n  - id: pass_word\n    when:\n      verb: pass_word\n    do:\n      - note:\n          id: note_private\n          to:\n            - pc_nell\n          text: The tide turns at dusk.\n",
    );
    write_nell(&pack, &[]);
    edit_save(world, &pack, |now| {
        seat(now, "pc_rowan", "pc_nell", "docks_street");
        sit(now, &["pc_nell", "pc_wren"]);
    });
    show(
        world,
        pack,
        vec![intent("nell-word", "pc_nell", "pass_word", Target::Empty)],
    );
}

#[then(expr = "spook verify passes on this pack")]
fn verify_passes_on_this_pack(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let output = support::spook(&["verify", &pack.to_string_lossy()]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "verify failed\n{stdout}{stderr}");
}

#[when(expr = "the empty table narrates and then Rowan narrates")]
fn the_empty_table_narrates_and_then_rowan_narrates(world: &mut World) {
    let pack = temp_pack("trigger-idle-narrate");
    add_verb(
        &pack,
        "\n  - id: quiet\n    when:\n      verb: quiet\n    do:\n      - narrate:\n          to: pc_rowan\n          text: The shop is quiet.\n  - id: back\n    when:\n      verb: back\n    do:\n      - narrate:\n          to: self\n          text: You are back.\n",
    );
    edit_save(world, &pack, |now| {
        now.presence.humans = 0;
        now.presence.actors_present.clear();
    });
    let mut kernel =
        spookery_engine::World::open(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    kernel.queue(intent("quiet", "pc_rowan", "quiet", Target::Empty));
    let wall = kernel.now().last_tick_at.clone();
    let quiet = kernel
        .play(RoundCause::Manual, &wall)
        .unwrap_or_else(|err| {
            panic!("quiet: {err}");
        });
    kernel.set_presence(spookery_engine::Presence {
        humans: 1,
        actors_present: vec!["pc_rowan".to_string()],
    });
    kernel.queue(intent("back", "pc_rowan", "back", Target::Empty));
    let wall = kernel.now().last_tick_at.clone();
    let back = kernel
        .play(RoundCause::Manual, &wall)
        .unwrap_or_else(|err| panic!("back: {err}"));
    world.told = quiet
        .lines
        .into_iter()
        .chain(back.lines)
        .map(|line| (line.to, line.text))
        .collect();
    world.pack = Some(pack);
}

#[given(expr = "a cast id world and a reveal of a missing fact")]
fn a_cast_id_world_and_a_bad_reveal(world: &mut World) {
    let pack = temp_pack("trigger-check-reveal");
    fs::write(
        pack.join("cast/world.md"),
        "\
---
schema: cast
id: world
kind: npc
display_name: World
facts: []
tick:
  eligible: false
---
Reserved.
",
    )
    .unwrap();
    add_verb(
        &pack,
        "\n  - id: bad\n    when:\n      verb: bad\n    do:\n      - reveal:\n          who: world\n          fact: missing_fact\n      - reveal:\n          who: self\n          fact: missing_fact\n",
    );
    edit_save(world, &pack, |_| {});
    world.pack = Some(pack);
}

fn seen_of(world: &World, viewer: &str) -> Vec<String> {
    world
        .seen
        .iter()
        .filter(|(id, _)| id == viewer)
        .flat_map(|(_, changes)| changes.clone())
        .collect()
}

fn is_tick_counter(key: &str) -> bool {
    key.starts_with("tick_last_act ")
        || key.starts_with("tick_idle_acts ")
        || key.starts_with("tick_live_acts ")
}

fn show(world: &mut World, pack: std::path::PathBuf, intents: Vec<Intent>) {
    let played = play_view(world, &pack, intents);
    world.summary = played.summary;
    world.seen = played.seen;
    world.pack = Some(pack);
}

fn put(now: &mut Now, id: &str, place: &str) {
    *now.actors.get_mut(id).unwrap().place_mut() = place.to_string();
}

fn sit(now: &mut Now, ids: &[&str]) {
    now.presence.humans = u32::try_from(ids.len()).unwrap();
    now.presence.actors_present = ids.iter().map(|id| (*id).to_string()).collect();
}

fn narrates(tick: &Value, text: &str, viewer: &str) -> bool {
    tick["effects"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|effect| {
            effect["class"] == "narrate"
                && effect["text"] == text
                && effect["to"]
                    .as_array()
                    .is_some_and(|to| to.iter().any(|id| id == viewer))
        })
}

fn add_bell_narrate(pack: &Path) {
    append(
        &pack.join("world/places/docks_shop.md"),
        "    locked_text: The stair door is locked.\n",
        "    locked_text: The stair door is locked.\nrules:\n  - id: bell\n    when:\n      change:\n        arrive: here\n        who: pc\n    do:\n      - flag:\n          who: self\n          key: heard_bell\n          value: true\n      - narrate:\n          to: here\n          text: The bell rings.\n",
    );
}

fn downed_pack(
    world: &mut World,
    label: &str,
    hp: i64,
    downed: bool,
    crosses: &str,
) -> std::path::PathBuf {
    let pack = temp_pack(label);
    append(
        &pack.join("rules/meters.yaml"),
        "    start: 1\n",
        "    start: 1\n  - id: hp\n    min: 0\n    base_cap: 12\n    max: 16\n    start: 8\n",
    );
    add_verb(
        &pack,
        "\n  - id: hit\n    when:\n      verb: hit\n      target: actor\n    do:\n      - meter:\n          who: target\n          key: hp\n          add: -4\n  - id: nick\n    when:\n      verb: nick\n      target: actor\n    do:\n      - meter:\n          who: target\n          key: hp\n          add: -1\n",
    );
    fs::write(
        pack.join("cast/briar.md"),
        "\
---
schema: cast
id: briar
kind: npc
display_name: Briar
facts: []
tick:
  eligible: false
  flags:
    - downed
---
Briar keeps their feet until they cannot.
",
    )
    .unwrap();
    fs::create_dir_all(pack.join("story")).unwrap();
    fs::write(
        pack.join("story/injury.yaml"),
        format!(
            "\
schema: story
id: injury
rules:
  - id: downed
    when:
      change:
        meter: hp
        crosses: \"{crosses}\"
    do:
      - flag:
          who: self
          key: downed
          value: true
"
        ),
    )
    .unwrap();
    edit_save(world, &pack, |now| {
        seat(now, "aldric", "briar", "docks_shop");
        let briar = now.actors.get_mut("briar").unwrap();
        briar.meters_mut().insert(
            "hp".to_string(),
            MeterLive {
                base: hp,
                offset: 0,
            },
        );
        if downed {
            briar.flags_mut().push("downed".to_string());
        }
        now.presence.humans = 1;
        now.presence.actors_present = vec!["pc_rowan".to_string()];
    });
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    pack
}

fn add_door(pack: &Path) {
    append(
        &pack.join("world/places/docks_street.md"),
        "    name: docks_shop\n",
        "    name: docks_shop\n  - to: docks_shop\n    name: door\n",
    );
}

fn add_rooms(pack: &Path) {
    append(
        &pack.join("world/places/docks_street.md"),
        "    name: docks_shop\n",
        "    name: docks_shop\n  - to: mira_rooms\n    name: rooms\n",
    );
}

fn add_bell(pack: &Path) {
    append(
        &pack.join("world/places/docks_shop.md"),
        "    locked_text: The stair door is locked.\n",
        "    locked_text: The stair door is locked.\nrules:\n  - id: bell\n    when:\n      change:\n        arrive: here\n        who: pc\n    do:\n      - flag:\n          who: self\n          key: heard_bell\n          value: true\n",
    );
}

fn add_verb(pack: &Path, yaml: &str) {
    let path = pack.join("rules/verbs.yaml");
    let mut text = fs::read_to_string(&path).unwrap();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(yaml);
    fs::write(path, text).unwrap();
}

fn flag_pcs(pack: &Path, flag: &str) {
    for name in ["pc-rowan.md", "pc-wren.md", "pc-harbormouse.md"] {
        append(
            &pack.join("cast").join(name),
            "  eligible: false\n",
            &format!("  eligible: false\n  flags:\n    - {flag}\n"),
        );
    }
}

fn write_nell(pack: &Path, flags: &[&str]) {
    let listed = flags
        .iter()
        .map(|flag| format!("    - {flag}\n"))
        .collect::<String>();
    let flags = if listed.is_empty() {
        String::new()
    } else {
        format!("  flags:\n{listed}")
    };
    fs::write(
        pack.join("cast/pc-nell.md"),
        format!(
            "\
---
schema: cast
id: pc_nell
kind: pc_template
display_name: Nell
facts: []
tick:
  eligible: false
{flags}---
Nell walks the docks.
"
        ),
    )
    .unwrap();
}

fn seat(now: &mut Now, from: &str, id: &str, place: &str) {
    let mut actor = now.actors.get(from).unwrap().clone();
    *actor.place_mut() = place.to_string();
    actor.flags_mut().clear();
    now.actors.insert(id.to_string(), actor);
    now.presence.humans = 1;
    now.presence.actors_present = vec![id.to_string()];
}

fn edit_save(world: &mut World, pack: &Path, edit: impl FnOnce(&mut Now)) {
    let path = pack.join("state/save.yaml");
    let mut now = load_save(&fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut now);
    remember_presence(world, pack, now.presence.clone());
    now.pack_rev = spookery_engine::pack_rev(pack).unwrap();
    fs::write(path, emit_save(&now).unwrap()).unwrap();
}

fn remember_presence(world: &mut World, pack: &Path, presence: spookery_engine::Presence) {
    world.seated.retain(|(path, _)| path != pack);
    world.seated.push((pack.to_path_buf(), presence));
}

fn remembered_presence(world: &World, pack: &Path) -> Option<spookery_engine::Presence> {
    world
        .seated
        .iter()
        .find(|(path, _)| path == pack)
        .map(|(_, presence)| presence.clone())
}

struct Played {
    now: Now,
    rolls: Vec<spookery_engine::MoveReport>,
    tick: Value,
    summary: Vec<String>,
    seen: Vec<(String, Vec<String>)>,
    told: Vec<(String, String)>,
}

fn play(
    world: &mut World,
    pack: &Path,
    intents: Vec<Intent>,
) -> (Now, Vec<spookery_engine::MoveReport>, Value) {
    let played = play_view(world, pack, intents);
    (played.now, played.rolls, played.tick)
}

fn play_view(scenario: &mut World, pack: &Path, intents: Vec<Intent>) -> Played {
    let mut round = spookery_engine::World::resume(pack).unwrap_or_else(|err| {
        let report = spookery_engine::check_pack(pack)
            .iter()
            .map(|diag| diag.report_line())
            .collect::<Vec<_>>()
            .join("\n");
        panic!("open: {err}\n{report}");
    });
    if let Some(presence) = remembered_presence(scenario, pack) {
        round.set_presence(presence);
    }
    let wall = round.now().last_tick_at.clone();
    for intent in intents {
        round.queue(intent);
    }
    let output = round
        .play(RoundCause::Manual, &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
    let tick = latest(pack);
    let told = output
        .lines
        .into_iter()
        .map(|line| (line.to, line.text))
        .collect();
    let seen = output
        .seen
        .iter()
        .map(|item| (item.viewer.clone(), item.changes.clone()))
        .collect();
    Played {
        now: round.now().clone(),
        rolls: output.reports,
        tick,
        summary: output.summary,
        seen,
        told,
    }
}

fn intent(id: &str, actor: &str, verb: &str, target: Target) -> Intent {
    Intent {
        id: id.to_string(),
        tick_index: 1,
        actor_id: actor.to_string(),
        actor_kind: ActorKind::Player,
        source: Source::Player,
        move_name: verb.to_string(),
        target,
        args: IntentArgs::default(),
        rationale_code: "test".to_string(),
    }
}

fn append(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).unwrap();
    let next = text.replacen(from, to, 1);
    assert_ne!(text, next, "missing {from} in {}", path.display());
    fs::write(path, next).unwrap();
}

fn load(pack: &Path) -> Now {
    let report = spookery_engine::testing::from_journal(pack, None, None)
        .unwrap_or_else(|err| panic!("{err}"));
    let text = std::str::from_utf8(&report.now_yaml).unwrap();
    load_save(text).unwrap()
}

fn latest(pack: &Path) -> Value {
    let names = spookery_engine::testing::journal::list_tick_keys(pack);
    let name = names
        .last()
        .unwrap_or_else(|| panic!("no tick was written"));
    let text = spookery_engine::testing::journal::read_record_line(pack, name)
        .unwrap_or_else(|err| panic!("{}", err.detail));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{name}: {err}"))
}
