use spookery_engine::testing::state_of;
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing::journal::list_round_keys;
use spookery_engine::testing::PendingAction;
use spookery_engine::{component, ActorId, Meter, State};

use super::common::{
    add_trapdoor, capture_play_stdout, edit_save, give_key, join_play, set_busy, wait_play_until,
    walk, write_play, EVENING,
};
use super::table;
use crate::support::{self, copy_docks};
use crate::World;

fn add_toll(pack: &Path) {
    let path = pack.join("cast/bram.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace(
        "  eligible: true\n",
        "  eligible: true\nflags:\n    - tolled\nrules:\n  - id: toll\n    when: habit\n    if:\n      - cycle:\n          name: day\n          from: 1260\n          to: 1320\n    do:\n      - flag:\n          of: self\n          key: tolled\n          value: true\n",
    );
    assert_ne!(text, next);
    fs::write(path, next).unwrap();
}

#[given(expr = "docks_street has no exit to mira_rooms")]
fn docks_street_has_no_exit_to_mira_rooms(world: &mut World) {
    let pack = copy_docks("no-exit");
    edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_street")
            .unwrap();
    });
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "rowan travels to mira_rooms from docks_street")]
fn rowan_travels_to_mira_rooms(world: &mut World) {
    let (mark, added) = table::player_send(world, "travel mira_rooms", "NO_EXIT");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the intent is refused with NO_EXIT and rowan stays put")]
fn refused_no_exit_and_stays(world: &mut World) {
    assert!(
        world.player_added.contains("NO_EXIT"),
        "{}",
        world.player_added
    );
    let now = state_of(table::pack(world));
    assert_eq!(now.actors().place["rowan"], "docks_street");
    assert_eq!(now.world_time, 1080);
    assert!(
        list_round_keys(table::pack(world)).is_empty(),
        "a refused exit played a round"
    );
}

#[given(expr = "the stairs exit needs brass_key and rowan has none")]
fn stairs_need_a_key(world: &mut World) {
    let pack = copy_docks("locked");
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs")]
fn rowan_travels_via_stairs(world: &mut World) {
    let before = state_of(table::pack(world));
    world.scratch = format!(
        "{} {} {}",
        before.world_time,
        before.round_index,
        before.actors().place["rowan"]
    );
    let (_mark, added) = table::player_send(world, "travel stairs", "locked");
    world.player_added = added;
}

#[then(expr = "rowan sees {string} and no state changes")]
fn rowan_sees_locked_and_nothing_changes(world: &mut World, text: String) {
    assert!(world.player_added.contains(&text), "{}", world.player_added);
    let now = state_of(table::pack(world));
    let mut parts = world.scratch.split(' ');
    assert_eq!(now.world_time.to_string(), parts.next().unwrap());
    assert_eq!(now.round_index.to_string(), parts.next().unwrap());
    assert_eq!(now.actors().place["rowan"], parts.next().unwrap());
    assert_eq!(
        now.actors()
            .items
            .get(&("rowan".into(), "brass_key".to_string()))
            .copied()
            .unwrap_or(0),
        0
    );
}

#[when(expr = "rowan travels via stairs holding brass_key")]
fn rowan_travels_holding_the_key(world: &mut World) {
    let pack = copy_docks("stairs-key");
    add_trapdoor(&pack);
    edit_save(&pack, give_key);
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    let (_mark, added) = table::player_send(world, "travel stairs", "mira_rooms");
    world.player_added = added;
    world.pack = Some(pack);
}

#[then(expr = "rowan is in mira_rooms and the 2 tick cost is stamped on the arrival")]
fn rowan_arrives_and_the_cost_is_stamped(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let now = state_of(&pack);
    assert_eq!(now.actors().place["rowan"], "mira_rooms");
    assert_eq!(now.world_time, 1085, "the round shortened or skipped");
    let round = crate::support::read_round(&pack, "round-0001");
    assert_eq!(arrival(&round, "rowan")["arrived_at"], 1082);
    assert_eq!(round["world_time_after"], 1085);
    assert_eq!(round["world_time_before"], 1080);
}

#[given(expr = "a place rule with location to cellar, teleport true")]
fn a_place_rule_teleports(world: &mut World) {
    let pack = copy_docks("teleport");
    add_trapdoor(&pack);
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap();
    let rules = "\nrules:\n  - id: slip\n    when:\n      verb: slip\n    do:\n      - location:\n          to: cellar\n          teleport: true\n";
    let text = text.replacen("\n---\n", &format!("{rules}---\n"), 1);
    assert!(text.contains("id: slip"), "{text}");
    fs::write(&path, text).unwrap();
    edit_save(&pack, |_| {});
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    world.pack = Some(pack);
}

#[then(expr = "the PC reaches cellar with no exit")]
fn the_pc_reaches_the_cellar(world: &mut World) {
    let (_mark, added) = table::player_send(world, "slip", "cellar");
    assert_eq!(
        state_of(table::pack(world)).actors().place["rowan"],
        "cellar",
        "{added}"
    );
}

#[then(expr = "the world is at tick 1300 and rowan is in mira_rooms")]
fn rowan_is_in_mira_rooms_at_tick_1300(world: &mut World) {
    let now = state_of(table::pack(world));
    assert_eq!(now.world_time, EVENING + 10);
    assert_eq!(now.actors().place["rowan"], "mira_rooms");
}

#[then(expr = "rowan's line shows {string}")]
fn rowans_line_shows(world: &mut World, text: String) {
    let needle = format!("\"place\":\"{text}\"");
    let added = table::wait_player_from(world, world.mark, &needle);
    assert!(added.contains(&needle), "{added}");
}

#[given(expr = "rowan arrives at tick 1300 and nell at tick 1295")]
fn two_arrivals(world: &mut World) {
    let pack = copy_docks("two-arrivals");
    fs::write(
        pack.join("cast/pc-nell.md"),
        "---\nschema: cast\nid: nell\nkind: pc_template\ndisplay_name: Nell\nidle:\n  eligible: false\n---\nNell.\n",
    )
    .unwrap();
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "rowan", EVENING + 10, "docks_shop.stairs");
        let walking = (EVENING + 5, walk("docks_shop.docks_street"));
        add_nell(now, "exit:docks_shop.docks_street", Some(walking));
    });
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    table::seat_actor(world, &pack, "nell");
    world.pack = Some(pack);
}

/// Add nell, a PC with wren's meters and nothing else, at `place`, busy with `walk`
/// until its tick when it is `Some`.
fn add_nell(now: &mut State, place: &str, walk: Option<(u64, PendingAction)>) {
    let nell = ActorId::from("nell");
    let meters: Vec<_> = (now.actors().meters.iter())
        .filter(|((owner, _), _)| *owner == *"wren")
        .map(|((_, key), meter)| (key.clone(), meter.clone()))
        .collect();
    let mut actors = now.actors_mut();
    actors.add_actor(&nell, component::ControlType::Pc, place);
    for (key, meter) in meters {
        actors
            .get_mut::<Meter>(&nell)
            .set(&key, Some(meter))
            .unwrap();
    }
    if let Some((until, pending)) = walk {
        actors
            .get_mut::<component::Act>(&nell)
            .set_busy(until, Some(pending))
            .unwrap();
    }
}

#[then(expr = "nell's arrival is applied before rowan's, in separate rounds")]
fn arrivals_are_in_time_order(world: &mut World) {
    let (_mark, _added) = table::player_send(world, "wait", "BUSY");
    let pack = table::pack(world).to_path_buf();
    let names = list_round_keys(&pack);
    assert!(names.len() >= 2, "{names:?}");
    let first = crate::support::read_round(&pack, &names[0]);
    let second = crate::support::read_round(&pack, &names[1]);
    assert_eq!(first["world_time_after"], EVENING + 5);
    assert_eq!(arrival(&first, "nell")["to"], "docks_street");
    assert_eq!(second["world_time_after"], EVENING + 10);
    assert_eq!(arrival(&second, "rowan")["to"], "mira_rooms");
    assert!(first["round_index"].as_u64() < second["round_index"].as_u64());
}

#[given(expr = "rounds are 5 ticks while seated and the stairs exit costs 2 ticks")]
fn seated_rounds_are_five_ticks(world: &mut World) {
    let pack = copy_docks("exact-tick");
    edit_save(&pack, |now| {
        give_key(now);
        now.world_time = EVENING;
    });
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs in the round starting at tick 1290")]
fn rowan_travels_in_the_evening_round(world: &mut World) {
    let (_mark, added) = table::player_send(world, "travel stairs", "mira_rooms");
    world.player_added = added;
}

#[then(expr = "the round still ends at tick 1295")]
fn the_round_still_ends_at_tick_1295(world: &mut World) {
    let round = crate::support::read_round(table::pack(world), "round-0001");
    assert_eq!(round["world_time_before"], EVENING);
    assert_eq!(round["world_time_after"], EVENING + 5);
}

#[then(expr = "rowan's arrival is recorded with arrived_at 1292 in that round's line")]
fn arrival_is_stamped(world: &mut World) {
    let round = crate::support::read_round(table::pack(world), "round-0001");
    assert_eq!(arrival(&round, "rowan")["arrived_at"], EVENING + 2);
    let stamped = round["effects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|effect| effect["kind"] == "location" && effect["arrived_at"] == EVENING + 2);
    assert!(stamped, "{round}");
}

#[then(expr = "a move from rowan is accepted from the round starting at tick 1295")]
fn the_next_round_accepts_a_move(world: &mut World) {
    let (_mark, added) = table::player_send(world, "wait", "\"kind\":");
    assert!(!added.contains("BUSY"), "{added}");
    let now = state_of(table::pack(world));
    assert_eq!(now.world_time, EVENING + 10);
    assert!(!now.actors().acts["rowan"].is_busy(now.world_time));
}

#[then(
    expr = "every round line shows world_time_after minus world_time_before equal to 5 seated or 60 unseated"
)]
fn rounds_never_shorten(_world: &mut World) {
    let seated = copy_docks("seated-length");
    let mut seated_world = spookery_engine::testing::resume(&seated);
    let one_human = spookery_engine::Presence {
        players: 1,
        actors_present: Vec::new(),
    };
    let wall = seated_world.state().last_round_at.clone();
    seated_world.play(&one_human, &wall).unwrap();
    drop(seated_world);
    let idle = copy_docks("idle-length");
    spookery_engine::play_pack_once(&idle).unwrap();
    let seated_round = crate::support::read_round(&seated, "round-0001");
    let idle_round = crate::support::read_round(&idle, "round-0001");
    assert_eq!(
        seated_round["world_time_after"].as_u64().unwrap()
            - seated_round["world_time_before"].as_u64().unwrap(),
        5
    );
    assert_eq!(
        idle_round["world_time_after"].as_u64().unwrap()
            - idle_round["world_time_before"].as_u64().unwrap(),
        60
    );
}

#[given(expr = "rowan and nell are seated and both busy, with no GM")]
fn both_busy_without_a_gm(world: &mut World) {
    let pack = copy_docks("fast-forward");
    add_toll(&pack);
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "rowan", EVENING + 10, "docks_shop.stairs");
    });
    fs::write(
        pack.join("cast/pc-nell.md"),
        "---\nschema: cast\nid: nell\nkind: pc_template\ndisplay_name: Nell\nidle:\n  eligible: false\n---\nNell.\n",
    )
    .unwrap();
    edit_save(&pack, |now| {
        add_nell(now, "docks_shop", None);
        set_busy(now, "nell", EVENING + 5, "docks_shop.docks_street");
    });
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    table::seat_actor(world, &pack, "nell");
    world.pack = Some(pack);
}

#[then(
    expr = "the table plays rounds to tick 1295 then tick 1300 without waiting on the wall clock"
)]
fn the_table_plays_rounds_through_both_arrivals(world: &mut World) {
    let (_mark, _added) = table::player_send(world, "wait", "BUSY");
    let pack = table::pack(world).to_path_buf();
    let names = list_round_keys(&pack);
    let ends: Vec<u64> = names
        .iter()
        .map(|name| {
            crate::support::read_round(&pack, name)["world_time_after"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert!(ends.contains(&(EVENING + 5)), "{ends:?}");
    assert!(ends.contains(&(EVENING + 10)), "{ends:?}");
}

#[then(expr = "an NPC habit due at tick 1295 runs in that round")]
fn the_habit_due_at_tick_1295_runs(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let round = list_round_keys(&pack)
        .into_iter()
        .find(|name| crate::support::read_round(&pack, name)["world_time_after"] == EVENING + 5)
        .unwrap_or_else(|| panic!("no round ending at tick 1295"));
    let steps = crate::support::read_round(&pack, &round)["extra_steps"].to_string();
    assert!(steps.contains("cast:bram/toll"), "{steps}");
}

#[given(expr = "a GM is seated and both PCs are busy")]
fn a_gm_and_two_busy_pcs(world: &mut World) {
    both_busy_without_a_gm(world);
    table::seat_gm(world);
}

#[then(expr = "nothing advances until the GM commits")]
fn nothing_advances_until_commit(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let before = state_of(&pack).world_time;
    let (_mark, added) = table::player_send(world, "wait", "BUSY");
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(state_of(&pack).world_time, before);
    let _ = table::gm_tool_call(world, 9, "commit", r#"{"include_round":true}"#);
    assert!(state_of(&pack).world_time > before);
}

#[given(expr = "busy_until is 200 rounds away and max_fastforward_rounds is 12")]
fn a_long_journey_and_the_cap(world: &mut World) {
    let pack = copy_docks("cap");
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "rowan", EVENING + 200 * 5, "docks_shop.stairs");
    });
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    world.pack = Some(pack);
}

#[then(expr = "12 rounds run and the rest continues on the next action")]
fn twelve_rounds_then_more(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    // BUSY is sent after the burst commits, so wait on the journal first.
    let mark = table::player_write(world, "wait");
    table::wait_round_count(&pack, 12);
    let added = table::wait_player_from(world, mark, "BUSY");
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(list_round_keys(&pack).len(), 12, "first burst");
    let mid = state_of(&pack).world_time;
    assert_eq!(mid, EVENING + 12 * 5);
    assert!(state_of(&pack).actors().acts["rowan"].is_busy(mid));
    let mark = table::player_write(world, "wait");
    table::wait_round_count(&pack, 24);
    let added = table::wait_player_from(world, mark, "BUSY");
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(list_round_keys(&pack).len(), 24, "second burst");
    assert_eq!(state_of(&pack).world_time, mid + 12 * 5);
}

#[then(expr = "spook verify passes over a fast-forwarded journey")]
fn verify_passes(world: &mut World) {
    let pack = copy_docks("verify-journey");
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "rowan", EVENING + 10, "docks_shop.stairs");
    });
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    let (_mark, _) = table::player_send(world, "wait", "BUSY");
    world.pack = Some(pack.clone());
    let arg = pack.to_str().unwrap();
    let output = support::run_spook(&["verify", arg]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("verified"), "{stdout}");
}

#[given(expr = "rowan visited the shop and mira_rooms in an earlier sitting")]
fn rowan_visited_both_rooms(world: &mut World) {
    let pack = copy_docks("map");
    add_trapdoor(&pack);
    edit_save(&pack, |now| {
        // No writer forgets a visit, so Rowan's visits are replaced in the save as the file
        // holds it.
        support::edit_as_saved(now, |save| {
            save["actors"]["visited"]["rowan"] = serde_json::json!({
                "docks_shop": ["docks_street", "stairs"],
                "mira_rooms": ["stairs"],
            });
        });
    });
    world.pack = Some(pack);
}

#[when(expr = "play joins the table again")]
fn play_joins_again(world: &mut World) {
    join_play(world, false);
}

/// The map is the table's: each place rowan's save says they visited, with the exits
/// rowan saw there last.
#[then(expr = "play's map lists the shop and mira_rooms and not the trapdoor")]
fn plays_map_lists_both_rooms(world: &mut World) {
    let mark = capture_play_stdout(world).len();
    write_play(world, "map");
    // Mira's Rooms is printed last, with one exit, so its finished exit line ends the map.
    wait_play_until(
        world,
        |stdout| stdout[mark..].contains("Mira's Rooms\n  ") && stdout.ends_with('\n'),
        "the map",
    );
    let map = capture_play_stdout(world)[mark..].to_string();
    assert!(map.starts_with("Docks Shop\n  "), "{map}");
    assert!(map.contains("Mira's Rooms\n  "), "{map}");
    assert!(!map.contains("trapdoor"), "{map}");
}

/// The location effect that lands `actor` at the end of a walk, stamped with its tick.
fn arrival(round: &serde_json::Value, actor: &str) -> serde_json::Value {
    let effects = round["effects"]
        .as_array()
        .unwrap_or_else(|| panic!("{round}"));
    let found = effects.iter().find(|effect| {
        effect["kind"] == "location" && effect["actor_id"] == actor && effect["arrived_at"].is_u64()
    });
    found
        .unwrap_or_else(|| panic!("no arrival for {actor}: {round}"))
        .clone()
}
