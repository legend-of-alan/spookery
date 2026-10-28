use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing::journal::list_tick_keys;

use super::common::{add_trapdoor, edit_save, give_key, set_busy, EVENING};
use super::table;
use crate::support::{self, temp_pack};
use crate::World;

fn pack_of(world: &World) -> std::path::PathBuf {
    world
        .pack
        .clone()
        .unwrap_or_else(|| table::pack(world).to_path_buf())
}

fn add_toll(pack: &Path) {
    let path = pack.join("cast/bram.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace(
        "  eligible: true\n",
        "  eligible: true\n  flags:\n    - tolled\nrules:\n  - id: toll\n    when: habit\n    if:\n      - hour:\n          from: 21\n          to: 22\n    do:\n      - flag:\n          who: self\n          key: tolled\n          value: true\n",
    );
    assert_ne!(text, next);
    fs::write(path, next).unwrap();
}

#[given(expr = "docks_street has no exit to mira_rooms")]
fn docks_street_has_no_exit_to_mira_rooms(world: &mut World) {
    let pack = temp_pack("no-exit");
    edit_save(&pack, |now| {
        *now.actors.get_mut("pc_rowan").unwrap().place_mut() = "docks_street".to_string();
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "rowan travels to mira_rooms from docks_street")]
fn rowan_travels_to_mira_rooms(world: &mut World) {
    let (mark, added) = table::player_send(world, "travel mira_rooms", "NO_EXIT");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the intent is rejected with NO_EXIT and rowan stays put")]
fn rejected_no_exit_and_stays(world: &mut World) {
    assert!(
        world.player_added.contains("NO_EXIT"),
        "{}",
        world.player_added
    );
    let now = table::now_of(&pack_of(world));
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "docks_street");
    assert_eq!(now.world_time, 1080);
    assert!(
        list_tick_keys(&pack_of(world)).is_empty(),
        "a refused exit ticked"
    );
}

#[given(expr = "the stairs exit needs brass_key and rowan has none")]
fn stairs_need_a_key(world: &mut World) {
    let pack = temp_pack("locked");
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs")]
fn rowan_travels_via_stairs(world: &mut World) {
    let before = table::now_of(&pack_of(world));
    world.scratch = format!(
        "{} {} {}",
        before.world_time,
        before.tick_index,
        before.actors.get("pc_rowan").unwrap().place()
    );
    let (_mark, added) = table::player_send(world, "travel stairs", "locked");
    world.player_added = added;
}

#[then(expr = "rowan sees {string} and no state changes")]
fn rowan_sees_locked_and_nothing_changes(world: &mut World, text: String) {
    assert!(world.player_added.contains(&text), "{}", world.player_added);
    let now = table::now_of(&pack_of(world));
    let mut parts = world.scratch.split(' ');
    assert_eq!(now.world_time.to_string(), parts.next().unwrap());
    assert_eq!(now.tick_index.to_string(), parts.next().unwrap());
    assert_eq!(
        now.actors.get("pc_rowan").unwrap().place(),
        parts.next().unwrap()
    );
    assert_eq!(now.actors.get("pc_rowan").unwrap().count("brass_key"), 0);
}

#[when(expr = "rowan travels via stairs holding brass_key")]
fn rowan_travels_holding_the_key(world: &mut World) {
    let pack = temp_pack("stairs-key");
    add_trapdoor(&pack);
    edit_save(&pack, give_key);
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    let (_mark, added) = table::player_send(world, "travel stairs", "mira_rooms");
    world.player_added = added;
    world.pack = Some(pack);
}

#[then(expr = "rowan is in mira_rooms and the 2 minute cost is stamped on the arrival")]
fn rowan_arrives_and_the_cost_is_stamped(world: &mut World) {
    let pack = pack_of(world);
    let now = table::now_of(&pack);
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "mira_rooms");
    assert_eq!(now.world_time, 1085, "the tick shortened or skipped");
    let tick = crate::support::read_tick(&pack, "tick-0001");
    assert_eq!(tick["arrivals"][0]["arrived_at"], 1082);
    assert_eq!(tick["world_time_after"], 1085);
    assert_eq!(tick["world_time_before"], 1080);
}

#[given(expr = "a place rule with location to cellar, teleport true")]
fn a_place_rule_teleports(world: &mut World) {
    let pack = temp_pack("teleport");
    add_trapdoor(&pack);
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap();
    let rules = "\nrules:\n  - id: slip\n    when:\n      verb: slip\n    do:\n      - location:\n          to: cellar\n          teleport: true\n";
    let text = text.replacen("\n---\n", &format!("{rules}---\n"), 1);
    assert!(text.contains("id: slip"), "{text}");
    fs::write(&path, text).unwrap();
    edit_save(&pack, |now| {
        now.places.insert(
            "cellar".to_string(),
            spookery_engine::testing::PlaceLive {
                items: Default::default(),
                notes: Vec::new(),
                actors: Vec::new(),
            },
        );
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[then(expr = "the PC reaches cellar with no exit")]
fn the_pc_reaches_the_cellar(world: &mut World) {
    let (_mark, added) = table::player_send(world, "slip", "cellar");
    assert_eq!(
        table::now_of(&pack_of(world))
            .actors
            .get("pc_rowan")
            .unwrap()
            .place(),
        "cellar",
        "{added}"
    );
}

#[then(expr = "the world is at 21:40 and rowan is in mira_rooms")]
fn rowan_has_arrived_by_half_past(world: &mut World) {
    let now = table::now_of(&pack_of(world));
    let rowan = now.actors.get("pc_rowan").unwrap();
    assert_eq!(now.world_time, EVENING + 10);
    assert_eq!(rowan.place(), "mira_rooms");
}

#[then(expr = "rowan's line shows {string}")]
fn rowans_line_shows(world: &mut World, text: String) {
    let needle = format!("\"place\":\"{text}\"");
    let added = table::wait_player_from(world, world.mark, &needle);
    assert!(added.contains(&needle), "{added}");
}

#[given(expr = "rowan arrives at 21:40 and nell at 21:35")]
fn two_arrivals(world: &mut World) {
    let pack = temp_pack("two-arrivals");
    fs::write(
        pack.join("cast/pc-nell.md"),
        "---\nschema: cast\nid: pc_nell\nkind: pc_template\ndisplay_name: Nell\nfacts: []\ntick:\n  eligible: false\n---\nNell.\n",
    )
    .unwrap();
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "pc_rowan", EVENING + 10, "mira_rooms", "stairs");
        let meters = now.actors.get("pc_rowan").unwrap().meters().clone();
        now.actors.insert(
            "pc_nell".to_string(),
            spookery_engine::testing::ActorLive::Pc {
                place: "docks_shop".to_string(),
                flags: Vec::new(),
                meters,
                inventory: Default::default(),
                wielded: None,
                known_facts: Vec::new(),
                visited: vec!["docks_shop".to_string()],
                act: {
                    let mut act = spookery_engine::testing::ActCounters::fresh();
                    act.busy_until = Some(EVENING + 5);
                    act.busy_for = Some(spookery_engine::testing::PendingAction {
                        rule: "verb:travel".to_string(),
                        verb: "travel".to_string(),
                        target: "docks_street".to_string(),
                        arrive: Some("docks_street".to_string()),
                        via: Some("docks_street".to_string()),
                        source: spookery_engine::Source::Player,
                    });
                    act
                },
            },
        );
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    table::seat_actor(world, &pack, "pc_nell");
    world.pack = Some(pack);
}

#[then(expr = "nell's arrival is applied before rowan's, in separate ticks")]
fn arrivals_are_in_time_order(world: &mut World) {
    let (_mark, _added) = table::player_send(world, "wait", "BUSY");
    let pack = pack_of(world);
    let names = list_tick_keys(&pack);
    assert!(names.len() >= 2, "{names:?}");
    let first = crate::support::read_tick(&pack, &names[0]);
    let second = crate::support::read_tick(&pack, &names[1]);
    assert_eq!(first["world_time_after"], EVENING + 5);
    assert_eq!(first["arrivals"][0]["actor_id"], "pc_nell");
    assert_eq!(second["world_time_after"], EVENING + 10);
    assert_eq!(second["arrivals"][0]["actor_id"], "pc_rowan");
    assert!(first["tick_index"].as_u64() < second["tick_index"].as_u64());
}

#[given(expr = "ticks are 5 minutes while seated and the stairs exit costs 2 minutes")]
fn seated_ticks_are_five_minutes(world: &mut World) {
    let pack = temp_pack("exact-minute");
    edit_save(&pack, |now| {
        give_key(now);
        now.world_time = EVENING;
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs in the tick starting 21:30")]
fn rowan_travels_in_the_evening_tick(world: &mut World) {
    let (_mark, added) = table::player_send(world, "travel stairs", "mira_rooms");
    world.player_added = added;
}

#[then(expr = "the tick still ends at 21:35")]
fn the_tick_ends_at_half(world: &mut World) {
    let tick = crate::support::read_tick(&pack_of(world), "tick-0001");
    assert_eq!(tick["world_time_before"], EVENING);
    assert_eq!(tick["world_time_after"], EVENING + 5);
}

#[then(expr = "rowan's arrival is recorded with arrived_at 21:32 in that tick's line")]
fn arrival_is_stamped(world: &mut World) {
    let tick = crate::support::read_tick(&pack_of(world), "tick-0001");
    assert_eq!(tick["arrivals"][0]["actor_id"], "pc_rowan");
    assert_eq!(tick["arrivals"][0]["arrived_at"], EVENING + 2);
    let stamped = tick["effects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|effect| effect["class"] == "location" && effect["arrived_at"] == EVENING + 2);
    assert!(stamped, "{tick}");
}

#[then(expr = "a move from rowan is accepted from the tick starting 21:35")]
fn the_next_tick_accepts_a_move(world: &mut World) {
    let (_mark, added) = table::player_send(world, "wait", "\"kind\":");
    assert!(!added.contains("BUSY"), "{added}");
    let now = table::now_of(&pack_of(world));
    assert_eq!(now.world_time, EVENING + 10);
    assert!(!now.actors.get("pc_rowan").unwrap().is_busy(now.world_time));
}

#[then(
    expr = "every tick line shows world_time_after minus world_time_before equal to 5 seated or 60 unseated"
)]
fn ticks_never_shorten(_world: &mut World) {
    let seated = temp_pack("seated-length");
    let mut seated_world = spookery_engine::World::open(&seated).unwrap();
    seated_world.set_presence(spookery_engine::testing::Presence {
        humans: 1,
        actors_present: Vec::new(),
    });
    let wall = seated_world.now().last_tick_at.clone();
    seated_world
        .play(spookery_engine::RoundCause::Manual, &wall)
        .unwrap();
    drop(seated_world);
    let idle = temp_pack("idle-length");
    spookery_engine::play_pack_once(&idle).unwrap();
    let seated_tick = crate::support::read_tick(&seated, "tick-0001");
    let idle_tick = crate::support::read_tick(&idle, "tick-0001");
    assert_eq!(
        seated_tick["world_time_after"].as_u64().unwrap()
            - seated_tick["world_time_before"].as_u64().unwrap(),
        5
    );
    assert_eq!(
        idle_tick["world_time_after"].as_u64().unwrap()
            - idle_tick["world_time_before"].as_u64().unwrap(),
        60
    );
}

#[given(expr = "rowan and nell are seated and both busy, with no GM")]
fn both_busy_without_a_gm(world: &mut World) {
    let pack = temp_pack("fast-forward");
    add_toll(&pack);
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "pc_rowan", EVENING + 10, "mira_rooms", "stairs");
    });
    fs::write(
        pack.join("cast/pc-nell.md"),
        "---\nschema: cast\nid: pc_nell\nkind: pc_template\ndisplay_name: Nell\nfacts: []\ntick:\n  eligible: false\n---\nNell.\n",
    )
    .unwrap();
    edit_save(&pack, |now| {
        let meters = now.actors.get("pc_rowan").unwrap().meters().clone();
        now.actors.insert(
            "pc_nell".to_string(),
            spookery_engine::testing::ActorLive::Pc {
                place: "docks_shop".to_string(),
                flags: Vec::new(),
                meters,
                inventory: Default::default(),
                wielded: None,
                known_facts: Vec::new(),
                visited: vec!["docks_shop".to_string()],
                act: spookery_engine::testing::ActCounters::fresh(),
            },
        );
        set_busy(now, "pc_nell", EVENING + 5, "docks_street", "docks_street");
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    table::seat_actor(world, &pack, "pc_nell");
    world.pack = Some(pack);
}

#[then(expr = "the shard ticks to 21:35 then 21:40 without waiting on the wall clock")]
fn the_shard_ticks_through_both_arrivals(world: &mut World) {
    let (_mark, _added) = table::player_send(world, "wait", "BUSY");
    let pack = pack_of(world);
    let names = list_tick_keys(&pack);
    let ends: Vec<u64> = names
        .iter()
        .map(|name| {
            crate::support::read_tick(&pack, name)["world_time_after"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert!(ends.contains(&(EVENING + 5)), "{ends:?}");
    assert!(ends.contains(&(EVENING + 10)), "{ends:?}");
}

#[then(expr = "an NPC habit due at 21:35 runs in that tick")]
fn the_habit_runs_at_half(world: &mut World) {
    let pack = pack_of(world);
    let tick = list_tick_keys(&pack)
        .into_iter()
        .find(|name| crate::support::read_tick(&pack, name)["world_time_after"] == EVENING + 5)
        .unwrap_or_else(|| panic!("no 21:35 tick"));
    let steps = crate::support::read_tick(&pack, &tick)["steps_run"].to_string();
    assert!(steps.contains("cast:bram/toll"), "{steps}");
}

#[given(expr = "a GM is seated and both PCs are busy")]
fn a_gm_and_two_busy_pcs(world: &mut World) {
    both_busy_without_a_gm(world);
    table::seat_gm(world);
}

#[then(expr = "nothing advances until the GM commits")]
fn nothing_advances_until_commit(world: &mut World) {
    let pack = pack_of(world);
    let before = table::now_of(&pack).world_time;
    let (_mark, added) = table::player_send(world, "wait", "BUSY");
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(table::now_of(&pack).world_time, before);
    let _ = table::gm_call(world, 9, "commit_round", r#"{"include_tick":true}"#);
    assert!(table::now_of(&pack).world_time > before);
}

#[given(expr = "busy_until is 200 ticks away and max_fastforward_ticks is 12")]
fn a_long_journey_and_the_cap(world: &mut World) {
    let pack = temp_pack("cap");
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "pc_rowan", EVENING + 200 * 5, "mira_rooms", "stairs");
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[then(expr = "12 ticks run and the rest continues on the next action")]
fn twelve_ticks_then_more(world: &mut World) {
    let pack = pack_of(world);
    // BUSY is sent after the burst commits, so wait on the journal first.
    let mark = table::player_write(world, "wait");
    table::wait_tick_count(&pack, 12);
    let added = table::wait_player_from(world, mark, "BUSY");
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(list_tick_keys(&pack).len(), 12, "first burst");
    let mid = table::now_of(&pack).world_time;
    assert_eq!(mid, EVENING + 12 * 5);
    assert!(table::now_of(&pack)
        .actors
        .get("pc_rowan")
        .unwrap()
        .is_busy(mid));
    let mark = table::player_write(world, "wait");
    table::wait_tick_count(&pack, 24);
    let added = table::wait_player_from(world, mark, "BUSY");
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(list_tick_keys(&pack).len(), 24, "second burst");
    assert_eq!(table::now_of(&pack).world_time, mid + 12 * 5);
}

#[then(expr = "spook verify passes over a fast-forwarded journey")]
fn verify_passes(world: &mut World) {
    let pack = temp_pack("verify-journey");
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "pc_rowan", EVENING + 10, "mira_rooms", "stairs");
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    let (_mark, _) = table::player_send(world, "wait", "BUSY");
    world.pack = Some(pack.clone());
    let arg = pack.to_str().unwrap();
    let output = support::spook(&["verify", arg]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("verified"), "{stdout}");
}

#[then(expr = "rowan's map lists mira_rooms and not the trapdoor")]
fn the_player_map_lists_the_room(world: &mut World) {
    let (_mark, map) = table::player_send(world, "map", "Mira");
    assert!(map.contains("mira_rooms") || map.contains("Mira"), "{map}");
    assert!(!map.contains("trapdoor"), "{map}");
}
