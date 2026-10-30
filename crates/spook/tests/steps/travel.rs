use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::testing::journal::list_round_keys;

use super::common::{add_trapdoor, edit_save, give_key, set_busy, EVENING};
use super::table;
use crate::support::{self, copy_docks};
use crate::World;

fn add_toll(pack: &Path) {
    let path = pack.join("cast/bram.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace(
        "  eligible: true\n",
        "  eligible: true\nflags:\n    - tolled\nrules:\n  - id: toll\n    when: habit\n    if:\n      - hour:\n          from: 21\n          to: 22\n    do:\n      - flag:\n          who: self\n          key: tolled\n          value: true\n",
    );
    assert_ne!(text, next);
    fs::write(path, next).unwrap();
}

#[given(expr = "docks_street has no exit to mira_rooms")]
fn docks_street_has_no_exit_to_mira_rooms(world: &mut World) {
    let pack = copy_docks("no-exit");
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
    let now = table::now_of(table::pack(world));
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "docks_street");
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
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs")]
fn rowan_travels_via_stairs(world: &mut World) {
    let before = table::now_of(table::pack(world));
    world.scratch = format!(
        "{} {} {}",
        before.world_time,
        before.round_index,
        before.actors.get("pc_rowan").unwrap().place()
    );
    let (_mark, added) = table::player_send(world, "travel stairs", "locked");
    world.player_added = added;
}

#[then(expr = "rowan sees {string} and no state changes")]
fn rowan_sees_locked_and_nothing_changes(world: &mut World, text: String) {
    assert!(world.player_added.contains(&text), "{}", world.player_added);
    let now = table::now_of(table::pack(world));
    let mut parts = world.scratch.split(' ');
    assert_eq!(now.world_time.to_string(), parts.next().unwrap());
    assert_eq!(now.round_index.to_string(), parts.next().unwrap());
    assert_eq!(
        now.actors.get("pc_rowan").unwrap().place(),
        parts.next().unwrap()
    );
    assert_eq!(
        now.actors.get("pc_rowan").unwrap().item_count("brass_key"),
        0
    );
}

#[when(expr = "rowan travels via stairs holding brass_key")]
fn rowan_travels_holding_the_key(world: &mut World) {
    let pack = copy_docks("stairs-key");
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
    let pack = table::pack(world).to_path_buf();
    let now = table::now_of(&pack);
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "mira_rooms");
    assert_eq!(now.world_time, 1085, "the round shortened or skipped");
    let round = crate::support::read_round(&pack, "round-0001");
    assert_eq!(round["arrivals"][0]["arrived_at"], 1082);
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
        table::now_of(table::pack(world))
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
    let now = table::now_of(table::pack(world));
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
    let pack = copy_docks("two-arrivals");
    fs::write(
        pack.join("cast/pc-nell.md"),
        "---\nschema: cast\nid: pc_nell\nkind: pc_template\ndisplay_name: Nell\nfacts: []\nidle:\n  eligible: false\n---\nNell.\n",
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
                act: spookery_engine::testing::Act {
                    busy_until: Some(EVENING + 5),
                    busy_for: Some(spookery_engine::testing::PendingAction {
                        verb: "travel".to_string(),
                        source: spookery_engine::Source::Player,
                        kind: spookery_engine::testing::PendingKind::Arrive {
                            place: "docks_street".to_string(),
                            via: "docks_street".to_string(),
                        },
                    }),
                    ..Default::default()
                },
            },
        );
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    table::seat_actor(world, &pack, "pc_nell");
    world.pack = Some(pack);
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
    assert_eq!(first["arrivals"][0]["actor_id"], "pc_nell");
    assert_eq!(second["world_time_after"], EVENING + 10);
    assert_eq!(second["arrivals"][0]["actor_id"], "pc_rowan");
    assert!(first["round_index"].as_u64() < second["round_index"].as_u64());
}

#[given(expr = "rounds are 5 minutes while seated and the stairs exit costs 2 minutes")]
fn seated_rounds_are_five_minutes(world: &mut World) {
    let pack = copy_docks("exact-minute");
    edit_save(&pack, |now| {
        give_key(now);
        now.world_time = EVENING;
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs in the round starting 21:30")]
fn rowan_travels_in_the_evening_round(world: &mut World) {
    let (_mark, added) = table::player_send(world, "travel stairs", "mira_rooms");
    world.player_added = added;
}

#[then(expr = "the round still ends at 21:35")]
fn the_round_ends_at_half(world: &mut World) {
    let round = crate::support::read_round(table::pack(world), "round-0001");
    assert_eq!(round["world_time_before"], EVENING);
    assert_eq!(round["world_time_after"], EVENING + 5);
}

#[then(expr = "rowan's arrival is recorded with arrived_at 21:32 in that round's line")]
fn arrival_is_stamped(world: &mut World) {
    let round = crate::support::read_round(table::pack(world), "round-0001");
    assert_eq!(round["arrivals"][0]["actor_id"], "pc_rowan");
    assert_eq!(round["arrivals"][0]["arrived_at"], EVENING + 2);
    let stamped = round["effects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|effect| effect["class"] == "location" && effect["arrived_at"] == EVENING + 2);
    assert!(stamped, "{round}");
}

#[then(expr = "a move from rowan is accepted from the round starting 21:35")]
fn the_next_round_accepts_a_move(world: &mut World) {
    let (_mark, added) = table::player_send(world, "wait", "\"kind\":");
    assert!(!added.contains("BUSY"), "{added}");
    let now = table::now_of(table::pack(world));
    assert_eq!(now.world_time, EVENING + 10);
    assert!(!now.actors.get("pc_rowan").unwrap().is_busy(now.world_time));
}

#[then(
    expr = "every round line shows world_time_after minus world_time_before equal to 5 seated or 60 unseated"
)]
fn rounds_never_shorten(_world: &mut World) {
    let seated = copy_docks("seated-length");
    let mut seated_world = spookery_engine::testing::resume(&seated);
    let one_human = spookery_engine::Presence {
        humans: 1,
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
        set_busy(now, "pc_rowan", EVENING + 10, "mira_rooms", "stairs");
    });
    fs::write(
        pack.join("cast/pc-nell.md"),
        "---\nschema: cast\nid: pc_nell\nkind: pc_template\ndisplay_name: Nell\nfacts: []\nidle:\n  eligible: false\n---\nNell.\n",
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
                act: spookery_engine::testing::Act::default(),
            },
        );
        set_busy(now, "pc_nell", EVENING + 5, "docks_street", "docks_street");
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    table::seat_actor(world, &pack, "pc_nell");
    world.pack = Some(pack);
}

#[then(expr = "the shard plays rounds to 21:35 then 21:40 without waiting on the wall clock")]
fn the_shard_plays_rounds_through_both_arrivals(world: &mut World) {
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

#[then(expr = "an NPC habit due at 21:35 runs in that round")]
fn the_habit_runs_at_half(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let round = list_round_keys(&pack)
        .into_iter()
        .find(|name| crate::support::read_round(&pack, name)["world_time_after"] == EVENING + 5)
        .unwrap_or_else(|| panic!("no 21:35 round"));
    let steps = crate::support::read_round(&pack, &round)["steps_run"].to_string();
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
    let before = table::now_of(&pack).world_time;
    let (_mark, added) = table::player_send(world, "wait", "BUSY");
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(table::now_of(&pack).world_time, before);
    let _ = table::gm_call(world, 9, "commit_round", r#"{"include_round":true}"#);
    assert!(table::now_of(&pack).world_time > before);
}

#[given(expr = "busy_until is 200 rounds away and max_fastforward_rounds is 12")]
fn a_long_journey_and_the_cap(world: &mut World) {
    let pack = copy_docks("cap");
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "pc_rowan", EVENING + 200 * 5, "mira_rooms", "stairs");
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
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
    let mid = table::now_of(&pack).world_time;
    assert_eq!(mid, EVENING + 12 * 5);
    assert!(table::now_of(&pack)
        .actors
        .get("pc_rowan")
        .unwrap()
        .is_busy(mid));
    let mark = table::player_write(world, "wait");
    table::wait_round_count(&pack, 24);
    let added = table::wait_player_from(world, mark, "BUSY");
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(list_round_keys(&pack).len(), 24, "second burst");
    assert_eq!(table::now_of(&pack).world_time, mid + 12 * 5);
}

#[then(expr = "spook verify passes over a fast-forwarded journey")]
fn verify_passes(world: &mut World) {
    let pack = copy_docks("verify-journey");
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
