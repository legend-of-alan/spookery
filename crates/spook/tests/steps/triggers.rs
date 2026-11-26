use std::fs;
use std::path::Path;
use std::sync::Arc;

use cucumber::{given, then, when};
use spookery::protocol::{self, ChangeView};
use spookery_engine::testing::world_of;
use spookery_engine::{self as engine, ecs, entity, Presence};
use spookery_engine::{component, emit_save, parse_save};

use super::common::{add_verb, flags_of, replace_in_file, write_tide_story};
use super::meters::write_world_meter_rows;
use crate::support::{self, copy_pack};
use crate::World;

#[when(expr = "nell travels via door into docks_shop")]
fn nell_travels_via_door(world: &mut World) {
    let pack = copy_pack("docks", "trigger-door");
    add_door(&pack);
    add_bell(&pack);
    add_verb(
        &pack,
        "\n  - id: blink\n    when:\n      verb_id: blink\n    do:\n      - location:\n          to: docks_shop\n          teleport: true\n",
    );
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_street");
    });
    play(
        world,
        &pack,
        vec![intent("nell-door", "nell", "travel", Some("door"))],
    );
    world.pack = Some(pack);
}

#[when(expr = "wren teleports into docks_shop by an author rule")]
fn wren_teleports_into_the_shop(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent("wren-blink", "wren", "blink", None)],
    );
}

#[given(expr = "briar has {int} hp and a story rule on hp crosses {word} sets flag downed")]
fn briar_has_hp_and_a_downed_rule(world: &mut World, hp: i64, crosses: String) {
    let pack = downed_pack(world, hp, &crosses);
    world.pack = Some(pack);
}

#[when(expr = "an attack removes 4 hp")]
fn an_attack_removes_4_hp(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent("hit-briar", "rowan", "hit", Some("briar"))],
    );
}

#[when(expr = "another round passes")]
fn another_round_passes(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent("nick-briar", "rowan", "nick", Some("briar"))],
    );
}

#[given(expr = "nell has flag found_key and arrives at mira_rooms")]
fn nell_arrives_with_the_key(world: &mut World) {
    let pack = copy_pack("docks", "trigger-ledger");
    add_rooms(&pack);
    write_world_meter_rows(&pack, &[("tide", 0, None)]);
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
        arrive_place_id: mira_rooms
        subject: pc
    if:
      - flag:
          of: self
          id: found_key
          is: true
    do:
      - flag:
          of: self
          id: found_ledger
          value: true
      - meter:
          of: world
          id: tide
          add: 1
",
    )
    .unwrap();
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_street");
        let nell = entity::Id::Actor("nell".into());
        now.entities_mut()
            .get_mut::<component::Flag>(&nell)
            .set("found_key", true)
            .unwrap();
        support::put_meter(now, &entity::Id::Global, "tide", 0);
    });
    play(
        world,
        &pack,
        vec![intent("nell-rooms", "nell", "travel", Some("rooms"))],
    );
    world.pack = Some(pack);
}

#[then(expr = "nell has flag found_ledger and the world's tide is {int}")]
fn nell_found_the_ledger(world: &mut World, tide: i64) {
    let now = world_of(world.pack.as_ref().unwrap());
    let nell = "nell".into();
    let flags = now.actors().get::<component::Flag>(&nell);
    assert!(flags.contains("found_ledger"), "nell has no found_ledger");
    let world_meters = now.globals().query::<&component::Meter>();
    let tide_meter = world_meters.iter().find_map(|meters| meters.get("tide"));
    let meter = tide_meter.expect("the world's tide");
    assert_eq!(meter.base + meter.offset, tide, "{meter:?}");
}

#[given(expr = "two rules that set each other's watched flag")]
fn two_rules_set_each_others_flag(world: &mut World) {
    let pack = copy_pack("docks", "trigger-loop");
    write_nell(&pack);
    add_verb(
        &pack,
        "\n  - id: raise\n    when:\n      verb_id: raise\n    do:\n      - flag:\n          of: self\n          id: sun\n          value: true\n",
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
        flag_id: sun
        becomes: true
    do:
      - flag:
          of: self
          id: moon
          value: true
      - flag:
          of: self
          id: sun
          value: false
      - meter:
          of: self
          id: vigor
          add: 1
  - id: moon_to_sun
    when:
      change:
        flag_id: moon
        becomes: true
    do:
      - flag:
          of: self
          id: sun
          value: true
      - flag:
          of: self
          id: moon
          value: false
      - meter:
          of: self
          id: vigor
          add: 1
",
    )
    .unwrap();
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_shop");
    });
    world.pack = Some(pack);
}

#[then(expr = "spook check warns of a cycle, and a round stops at depth 4 and still commits")]
async fn check_warns_of_a_cycle_and_the_round_stops(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let (status, stdout, stderr) = support::run_spook(&["check", &pack.to_string_lossy()]).await;
    assert!(status.success(), "check failed\n{stdout}{stderr}");
    assert!(stderr.contains("cycle"), "{stderr}");
    assert!(stderr.contains("WARNING"), "{stderr}");
    a_round_stops_at_depth_4(world);
}

fn a_round_stops_at_depth_4(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let (now, _reports) = play(
        world,
        &pack,
        vec![intent("raise-sun", "nell", "raise", None)],
    );
    let nell = entity::Id::Actor("nell".into());
    let vigor = now.entities().get::<component::Meter>(&nell).value("vigor");
    assert_eq!(vigor, Ok(5), "four passes add 4");
}

#[given(expr = "a rule on an arrival narrates to the subject")]
fn an_arrival_narrates_to_the_subject(world: &mut World) {
    let pack = copy_pack("docks", "trigger-leak-self");
    fs::create_dir_all(pack.join("story")).unwrap();
    fs::write(
        pack.join("story/greet.yaml"),
        "\
schema: story
id: greet
rules:
  - id: tell
    when:
      change:
        arrive_place_id: docks_shop
    do:
      - narrate:
          to: self
          text: You came in.
",
    )
    .unwrap();
    edit_table(world, &pack, |_, _| {});
    world.pack = Some(pack);
}

#[when(expr = "nell enters the bog")]
fn nell_enters_the_bog(world: &mut World) {
    let pack = copy_pack("docks", "trigger-bog");
    support::append(
        &pack.join("world/map.yaml"),
        "  - path: docks_street -> bog\n",
    );
    fs::write(
        pack.join("world/places/bog.md"),
        "\
---
schema: place
id: bog
name: Bog
rules:
  - id: sink
    when:
      change:
        arrive_place_id: here
        subject: pc
    check:
      total: 1d6 + self.vigor
      success_at: 100
    success:
      - meter:
          of: self
          id: vigor
          add: 0
    miss:
      - meter:
          of: self
          id: vigor
          add: -1
---
The ground gives.
",
    )
    .unwrap();
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_street");
    });
    let (_now, rolls) = play(
        world,
        &pack,
        vec![intent(
            "nell-bog",
            "nell",
            "travel",
            Some("docks_street.bog"),
        )],
    );
    let faces = rolls.iter().any(|report| match report {
        engine::roll::Report::Rolled { actor_id, .. }
        | engine::roll::Report::Refused { actor_id, .. } => actor_id == "nell",
    });
    assert!(faces, "nell did not roll\n{rolls:?}");
    world.pack = Some(pack);
}

#[then(expr = "that arrival rolled stamina and spook verify reproduces it")]
async fn the_bog_roll_reproduces(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let now = world_of(pack);
    let nell = "nell".into();
    let vigor = now
        .actors()
        .get::<component::Meter>(&nell)
        .get("vigor")
        .unwrap();
    assert_eq!(vigor.offset, -1, "the miss band did not spend vigor");
    let (status, stdout, stderr) = support::run_spook(&["verify", &pack.to_string_lossy()]).await;
    assert!(status.success(), "verify failed\n{stdout}{stderr}");
}

#[when(expr = "nell travels into the shop and the bell narrates")]
fn nell_travels_and_the_bell_narrates(world: &mut World) {
    let pack = copy_pack("docks", "trigger-narrate");
    add_door(&pack);
    add_bell_narrate(&pack);
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_street");
    });
    let played = play_round(
        world,
        &pack,
        vec![intent("nell-door", "nell", "travel", Some("door"))],
    );
    world.summary = played.changed_keys;
    world.seen = played.seen;
    world.told = played.lines;
    world.pack = Some(pack);
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

/// A trapdoor from the street to a cellar that `found_trapdoor` reveals, and an `uncover` verb
/// that sets that flag on `entity`, `self` or `world`.
#[given(
    expr = "the street has an exit revealed by the found_trapdoor flag, which uncover sets on {word}"
)]
fn the_street_has_a_trapdoor(world: &mut World, entity: String) {
    let pack = copy_pack("docks", "trigger-uncover");
    support::append(
        &pack.join("world/map.yaml"),
        "  - path: docks_street -> cellar\n    id: trapdoor\n    revealed_by: found_trapdoor\n",
    );
    fs::write(
        pack.join("world/places/cellar.md"),
        "---\nschema: place\nid: cellar\nname: Cellar\n---\nUnder the street.\n",
    )
    .unwrap();
    add_verb(
        &pack,
        &format!("\n  - id: uncover\n    when:\n      verb_id: uncover\n    do:\n      - flag:\n          of: {entity}\n          id: found_trapdoor\n          value: true\n"),
    );
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_street");
    });
    world.pack = Some(pack);
}

#[when(expr = "nell uncovers the trapdoor")]
fn nell_uncovers_the_trapdoor(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent("nell-uncover", "nell", "uncover", None)],
    );
    world.pack = Some(pack);
}

#[then(regex = r"^(nell|wren) (can|cannot) see the trapdoor$")]
fn the_viewer_sees_the_trapdoor(world: &mut World, viewer: String, can_or_cannot: String) {
    let pack = world.pack.as_ref().unwrap();
    let world_now = spookery_engine::Session::peek(pack).unwrap();
    let exits = world_now
        .perceive(&Presence::default(), &viewer)
        .unwrap()
        .place
        .exits;
    let sees = exits.iter().any(|exit| exit.path_id == "trapdoor");
    let should_see = can_or_cannot == "can";
    assert_eq!(
        sees, should_see,
        "{viewer} {can_or_cannot} see it: {exits:?}"
    );
}

#[given(expr = "a verb sets world flag tide_turned")]
fn a_verb_sets_the_tide(world: &mut World) {
    let pack = copy_pack("docks", "trigger-tide");
    add_verb(
        &pack,
        "
  - id: turn_tide
    when:
      verb_id: turn_tide
    do:
      - flag:
          of: world
          id: tide_turned
          value: true
",
    );
    world.pack = Some(pack);
}

#[given(
    expr = "a change rule watches tide_turned become true and adds 1 to meter tides of {string}"
)]
fn a_rule_counts_tides(world: &mut World, of: String) {
    let pack = world.pack.as_ref().unwrap();
    write_world_meter_rows(pack, &[("tides", 0, None)]);
    write_tide_story(
        pack,
        &format!(
            "    do:
      - meter:
          of: {of}
          id: tides
          add: 1
"
        ),
    );
}

/// Add nell on the street beside rowan in the shop, seat both, and migrate the save.
#[given(expr = "rowan and nell are seated")]
fn rowan_and_nell_are_seated(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_street");
        set_present(seated, &["rowan", "nell"]);
    });
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
}

#[when(expr = "nell sets the tide")]
fn nell_sets_the_tide(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    play(
        world,
        &pack,
        vec![intent("nell-tide", "nell", "turn_tide", None)],
    );
}

/// The flags the world holds, in name order, for a failure message.
pub(crate) fn world_flags(world: &ecs::World) -> Vec<String> {
    let globals = world.globals().query::<&component::Flag>();
    let flags = globals.iter().flat_map(|entry| entry.iter_with_id());
    flags.map(|(_, flag, ())| flag.to_string()).collect()
}

#[then(expr = "the world has flag {word}")]
fn the_world_has_flag(world: &mut World, flag: String) {
    let now = world_of(world.pack.as_ref().unwrap());
    let flags = now.entities().get::<component::Flag>(&entity::Id::Global);
    assert!(flags.contains(&flag), "{:?}", world_flags(&now));
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
    let pack = copy_pack("docks", "trigger-round-counter");
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_street");
    });
    play_and_keep_summaries(world, pack, Vec::new());
}

#[then(expr = "the GM summary lists a round counter change")]
fn the_gm_summary_lists_a_round_counter(world: &mut World) {
    assert!(
        world.summary.iter().any(|item| is_act_counter(item)),
        "no round counter\n{:?}",
        world.summary
    );
}

#[then(expr = "{string}'s summary lists no round counter change")]
fn the_viewer_summary_omits_round_counters(world: &mut World, viewer: String) {
    let changes = seen_of(world, &viewer);
    assert!(
        changes.iter().all(|item| !is_act_counter(item)),
        "{viewer} saw a round counter\n{changes:?}"
    );
}

#[when(expr = "an item is taken from docks_shop")]
fn an_item_is_taken_from_the_shop(world: &mut World) {
    let pack = copy_pack("docks", "trigger-place-item");
    edit_table(world, &pack, |_, seated| {
        set_present(seated, &["rowan"]);
    });
    play_and_keep_summaries(
        world,
        pack,
        vec![intent("rowan-take", "rowan", "take", Some("lamp_oil"))],
    );
}

#[when(expr = "rowan takes an item while nell is in the same room")]
fn rowan_takes_an_item_beside_nell(world: &mut World) {
    let pack = copy_pack("docks", "trigger-actor-item");
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_shop");
        set_present(seated, &["rowan", "nell", "wren"]);
    });
    play_and_keep_summaries(
        world,
        pack,
        vec![intent("rowan-take", "rowan", "take", Some("lamp_oil"))],
    );
}

/// `of: here` is known only at play time, so the pack check lets the habit through.
#[given(expr = "Mira has a habit that adds 1 to {string} of her place, a meter only she carries")]
fn mira_has_a_place_meter_habit(world: &mut World, meter: String) {
    let pack = copy_pack("docks", "habit-meter");
    replace_in_file(
        &pack.join("cast/mira.md"),
        "rules:\n  - id: leave_door_note\n",
        &format!("rules:\n  - id: mark_restock\n    when: habit\n    do:\n      - meter:\n          of: here\n          id: {meter}\n          add: 1\n  - id: leave_door_note\n"),
    );
    edit_table(world, &pack, |_, _| {});
    world.pack = Some(pack);
}

#[given(expr = "rowan and wren are seated in the shop with Mira")]
fn rowan_and_wren_are_seated_with_mira(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    edit_table(world, &pack, |now, seated| {
        set_place(now, "mira", "docks_shop");
        set_place(now, "wren", "docks_shop");
        set_present(seated, &["rowan", "wren"]);
    });
}

#[when(expr = "the GM queues Mira's {string} habit and commits")]
fn the_gm_queues_miras_habit(world: &mut World, habit: String) {
    let pack = world.pack.clone().unwrap();
    play_and_keep_summaries(world, pack, vec![intent("gm-habit", "mira", &habit, None)]);
}

#[when(expr = "an empty round passes")]
fn an_empty_round_passes(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    play_and_keep_summaries(world, pack, Vec::new());
}

#[then(expr = "no place has a {string} meter")]
fn no_place_has_the_meter(world: &mut World, meter: String) {
    let now = world_of(world.pack.as_ref().unwrap());
    let places = now.places().query::<&component::Meter>();
    assert!(!places.iter().any(|meters| meters.contains(&meter)));
}

#[when(expr = "nell travels from docks_street into docks_shop")]
fn nell_travels_from_the_street_into_the_shop(world: &mut World) {
    let pack = copy_pack("docks", "trigger-both-rooms");
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_street");
        set_place(now, "harbormouse", "mira_rooms");
        set_present(seated, &["nell"]);
    });
    play_and_keep_summaries(
        world,
        pack,
        vec![intent(
            "nell-shop",
            "nell",
            "travel",
            Some("docks_shop.docks_street"),
        )],
    );
}

#[when(expr = "a message hidden from wren is written")]
fn a_message_hidden_from_wren_is_written(world: &mut World) {
    let pack = copy_pack("docks", "trigger-message");
    add_verb(
        &pack,
        "\n  - id: pass_word\n    when:\n      verb_id: pass_word\n    do:\n      - note:\n          id: note_private\n          to:\n            - nell\n          text: The tide turns at dusk.\n",
    );
    write_nell(&pack);
    edit_table(world, &pack, |now, seated| {
        insert_actor_copy(now, seated, ("rowan", "nell"), "docks_street");
        set_present(seated, &["nell", "wren"]);
    });
    play_and_keep_summaries(
        world,
        pack,
        vec![intent("nell-word", "nell", "pass_word", None)],
    );
}

#[then(expr = "spook verify passes on this pack")]
async fn verify_passes_on_this_pack(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let (status, stdout, stderr) = support::run_spook(&["verify", &pack.to_string_lossy()]).await;
    assert!(status.success(), "verify failed\n{stdout}{stderr}");
}

#[when(expr = "the empty table narrates and then Rowan narrates")]
fn the_empty_table_narrates_and_then_rowan_narrates(world: &mut World) {
    let pack = copy_pack("docks", "trigger-idle-narrate");
    add_verb(
        &pack,
        "\n  - id: quiet\n    when:\n      verb_id: quiet\n    do:\n      - narrate:\n          to: rowan\n          text: The shop is quiet.\n  - id: back\n    when:\n      verb_id: back\n    do:\n      - narrate:\n          to: self\n          text: You are back.\n",
    );
    edit_table(world, &pack, |_, _| {});
    let mut session = spookery_engine::testing::resume(&pack);
    queue(&mut session, intent("quiet", "rowan", "quiet", None));
    let wall = session.world().last_round_at.clone();
    let quiet = session
        .play(&Presence::default(), &wall)
        .unwrap_or_else(|err| {
            panic!("quiet: {err}");
        });
    let rowan = Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    };
    queue(&mut session, intent("back", "rowan", "back", None));
    let wall = session.world().last_round_at.clone();
    let back = session
        .play(&rowan, &wall)
        .unwrap_or_else(|err| panic!("back: {err}"));
    world.told = quiet
        .lines
        .into_iter()
        .chain(back.lines)
        .map(|line| (line.to.into(), line.text))
        .collect();
    world.pack = Some(pack);
}

pub(crate) fn seen_of(world: &World, viewer: &str) -> Vec<String> {
    world
        .seen
        .iter()
        .filter(|(id, _)| id == viewer)
        .flat_map(|(_, changes)| changes.clone())
        .collect()
}

fn is_act_counter(key: &str) -> bool {
    key.starts_with("actor_last_act_round ")
        || key.starts_with("actor_idle_acts_this_window ")
        || key.starts_with("actor_live_acts_this_window ")
}

fn play_and_keep_summaries(
    world: &mut World,
    pack: std::path::PathBuf,
    intents: Vec<protocol::QueueArgs>,
) {
    let played = play_round(world, &pack, intents);
    world.summary = played.changed_keys;
    world.seen = played.seen;
    world.pack = Some(pack);
}

pub(crate) fn set_place(now: &mut ecs::World, id: &str, place: &str) {
    now.actors_mut()
        .get_mut::<component::Place>(&id.into())
        .set(place)
        .unwrap();
}

/// Marks these PCs present, sorted the way the host's seats are.
pub(crate) fn set_present(presence: &mut Presence, ids: &[&str]) {
    presence.players = u32::try_from(ids.len()).unwrap();
    presence.actors_present = ids.iter().map(|id| (*id).into()).collect();
    presence.actors_present.sort();
}

fn add_bell_narrate(pack: &Path) {
    support::insert_before_fence(
        &pack.join("world/places/docks_shop.md"),
        "rules:\n  - id: bell\n    when:\n      change:\n        arrive_place_id: here\n        subject: pc\n    do:\n      - flag:\n          of: self\n          id: heard_bell\n          value: true\n      - narrate:\n          to: here\n          text: The bell rings.\n",
    );
}

fn downed_pack(world: &mut World, hp: i64, crosses: &str) -> std::path::PathBuf {
    let pack = copy_pack("docks", "trigger-downed");
    add_verb(
        &pack,
        "\n  - id: hit\n    when:\n      verb_id: hit\n      target: actor\n    do:\n      - meter:\n          of: target\n          id: hp\n          add: -4\n  - id: nick\n    when:\n      verb_id: nick\n      target: actor\n    do:\n      - meter:\n          of: target\n          id: hp\n          add: -1\n",
    );
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
        meter_id: hp
        crosses: \"{crosses}\"
    do:
      - flag:
          of: self
          id: downed
          value: true
"
        ),
    )
    .unwrap();
    edit_table(world, &pack, |now, seated| {
        set_place(now, "briar", "docks_shop");
        let briar = entity::Id::Actor("briar".into());
        support::put_meter(now, &briar, "hp", hp);
        set_present(seated, &["rowan"]);
    });
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    pack
}

fn add_door(pack: &Path) {
    let door = "  - path: docks_street -> docks_shop\n    id: door\n";
    support::append(&pack.join("world/map.yaml"), door);
}

fn add_rooms(pack: &Path) {
    let rooms = "  - path: docks_street -> mira_rooms\n    id: rooms\n";
    support::append(&pack.join("world/map.yaml"), rooms);
}

fn add_bell(pack: &Path) {
    support::insert_before_fence(
        &pack.join("world/places/docks_shop.md"),
        "rules:\n  - id: bell\n    when:\n      change:\n        arrive_place_id: here\n        subject: pc\n    do:\n      - flag:\n          of: self\n          id: heard_bell\n          value: true\n",
    );
}

fn write_nell(pack: &Path) {
    fs::write(
        pack.join("cast/pc-nell.md"),
        "\
---
schema: cast
id: nell
control_type: pc_template
name: Nell
idle:
  eligible: false
---
Nell walks the docks.
",
    )
    .unwrap();
}

fn insert_actor_copy(
    now: &mut ecs::World,
    seated: &mut Presence,
    (from, id): (&str, &str),
    place: &str,
) {
    let (from, to) = (from.into(), id.into());
    now.actors_mut().copy_actor(&from, &to, place).unwrap();
    let flags = flags_of(now, id);
    let copy = entity::Id::Actor(to);
    let mut actors = now.entities_mut();
    for key in flags {
        actors
            .get_mut::<component::Flag>(&copy)
            .set(&key, false)
            .unwrap();
    }
    actors
        .get_mut::<component::Meter>(&copy)
        .set("assassination_debt", None)
        .unwrap();
    set_present(seated, &[id]);
}

/// Edit the save, and remember who the edit seats for the rounds this pack plays.
pub(crate) fn edit_table(
    world: &mut World,
    pack: &Path,
    edit: impl FnOnce(&mut ecs::World, &mut Presence),
) {
    let path = pack.join("state/save.yaml");
    let save = parse_save(&fs::read_to_string(&path).unwrap()).unwrap();
    let canon = spookery_engine::load_canon(pack).unwrap_or_else(|err| panic!("canon: {err:?}"));
    let mut now = ecs::World::new(save, Arc::new(canon));
    let mut seated = Presence::default();
    edit(&mut now, &mut seated);
    remember_presence(world, pack, seated);
    now.pack_rev = spookery_engine::pack_rev(pack).unwrap();
    fs::write(path, emit_save(&now).unwrap()).unwrap();
}

fn remember_presence(world: &mut World, pack: &Path, presence: Presence) {
    world.seated.retain(|(path, _)| path != pack);
    world.seated.push((pack.to_path_buf(), presence));
}

fn remembered_presence(world: &World, pack: &Path) -> Option<Presence> {
    world
        .seated
        .iter()
        .find(|(path, _)| path == pack)
        .map(|(_, presence)| presence.clone())
}

pub(crate) struct PlayedRound {
    now: ecs::World,
    reports: Vec<engine::roll::Report>,
    pub(crate) changed_keys: Vec<String>,
    pub(crate) seen: Vec<(String, Vec<String>)>,
    pub(crate) lines: Vec<(String, String)>,
    /// Each viewer's changes, as the engine reports them.
    pub(crate) changes: Vec<(String, Vec<ChangeView>)>,
}

fn play(
    world: &mut World,
    pack: &Path,
    intents: Vec<protocol::QueueArgs>,
) -> (ecs::World, Vec<engine::roll::Report>) {
    let played = play_round(world, pack, intents);
    (played.now, played.reports)
}

pub(crate) fn play_round(
    scenario: &mut World,
    pack: &Path,
    intents: Vec<protocol::QueueArgs>,
) -> PlayedRound {
    let mut session = super::rules::open(pack);
    let presence = remembered_presence(scenario, pack).unwrap_or_default();
    let wall = session.world().last_round_at.clone();
    for intent in intents {
        queue(&mut session, intent);
    }
    let output = session
        .play(&presence, &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
    let told = output
        .lines
        .into_iter()
        .map(|line| (line.to.into(), line.text))
        .collect();
    let seen = output
        .seen
        .iter()
        .map(|item| (item.viewer.to_string(), item.changed_keys.clone()))
        .collect();
    let changes = output
        .seen
        .iter()
        .map(|item| (item.viewer.to_string(), item.changes.clone()))
        .collect();
    PlayedRound {
        now: session.world().clone(),
        reports: output.reports,
        changed_keys: output.changed_keys,
        seen,
        lines: told,
        changes,
    }
}

/// The GM's `queue` command: `actor` plays `verb` at `target`, under the id `id`.
pub(crate) fn intent(
    id: &str,
    actor_id: &str,
    verb: &str,
    target: Option<&str>,
) -> protocol::QueueArgs {
    protocol::QueueArgs {
        id: Some(id.to_string()),
        round_index: None,
        actor_id: Some(actor_id.to_string()),
        move_id: Some(verb.to_string()),
        target: target.map(str::to_string),
        message_to: None,
        rationale_code: "test".to_string(),
    }
}

/// The GM queues `intent` under the id it names.
fn queue(session: &mut spookery_engine::Session, intent: protocol::QueueArgs) {
    session
        .queue_gm_intent(&spookery_engine::Presence::default(), intent, || {
            panic!("the GM chose the id")
        })
        .unwrap_or_else(|code| panic!("queue: {code}"));
}
