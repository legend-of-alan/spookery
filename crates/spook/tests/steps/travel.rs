use spookery_engine::testing::{is_busy, place_of, world_of};
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::{component, ecs, ActorId, Meter};

use super::common::{
    add_trapdoor, capture_play_stdout, edit_save, give_key, play_as_pc, set_busy, wait_play_until,
    write_play, EVENING,
};
use super::table;
use crate::support::{self, copy_pack};
use crate::World;

fn add_toll(pack: &Path) {
    let path = pack.join("cast/bram.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace(
        "  eligible: true\n",
        "  eligible: true\nflags:\n    - tolled\nrules:\n  - id: toll\n    when: habit\n    if:\n      - cycle:\n          id: day\n          from: 1260\n          to: 1320\n    do:\n      - flag:\n          of: self\n          key: tolled\n          value: true\n",
    );
    assert_ne!(text, next);
    fs::write(path, next).unwrap();
}

#[given(expr = "docks_street has no exit to mira_rooms")]
async fn docks_street_has_no_exit_to_mira_rooms(world: &mut World) {
    let pack = copy_pack("docks", "no-exit");
    edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_street")
            .unwrap();
    });
    table::listen(world, &pack).await;
    table::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "rowan travels to mira_rooms from docks_street")]
async fn rowan_travels_to_mira_rooms(world: &mut World) {
    let (mark, added) = table::player_send(world, "travel mira_rooms", "NO_EXIT").await;
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
    let now = world_of(table::pack(world));
    assert_eq!(place_of(&now, "rowan"), "docks_street");
    assert_eq!(now.world_time, 1080);
}

#[given(
    regex = r"^rowan stands in (docks_shop|mira_rooms), the stairs exit needs brass_key, rowan has none, and travel walks (at once|inside a nested check)$"
)]
async fn stairs_need_a_key(world: &mut World, place: String, walk: String) {
    let pack = copy_pack("docks", "locked");
    if place != "docks_shop" {
        edit_save(&pack, |now| {
            let rowan = &"rowan".into();
            (now.actors_mut().get_mut::<component::Place>(rowan))
                .set(&place)
                .unwrap();
        });
    }
    if walk == "inside a nested check" {
        super::common::replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "    do:\n      - location:\n          via: target\n",
            "    do:\n      - check:\n          total: 1\n          success_at: 1\n          success:\n            - location:\n                via: target\n",
        );
        support::stamp_pack_rev(&pack);
    }
    table::listen(world, &pack).await;
    table::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs")]
async fn rowan_travels_via_stairs(world: &mut World) {
    let before = world_of(table::pack(world));
    world.scratch = format!(
        "{} {} {}",
        before.world_time,
        before.round_index,
        place_of(&before, "rowan")
    );
    let (_mark, added) = table::player_send(world, "travel stairs", "locked").await;
    world.player_added = added;
}

#[then(expr = "rowan sees {string} and no state changes")]
fn rowan_sees_locked_and_nothing_changes(world: &mut World, text: String) {
    assert!(world.player_added.contains(&text), "{}", world.player_added);
    let now = world_of(table::pack(world));
    let mut parts = world.scratch.split(' ');
    assert_eq!(now.world_time.to_string(), parts.next().unwrap());
    assert_eq!(now.round_index.to_string(), parts.next().unwrap());
    assert_eq!(place_of(&now, "rowan"), parts.next().unwrap());
    assert_eq!(
        now.actors()
            .get::<component::Item>(&"rowan".into())
            .get("brass_key")
            .copied()
            .unwrap_or(0),
        0
    );
}

#[given(expr = "a place rule with location to cellar, teleport true")]
async fn a_place_rule_teleports(world: &mut World) {
    let pack = copy_pack("docks", "teleport");
    add_trapdoor(&pack);
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap();
    let rules = "\nrules:\n  - id: slip\n    when:\n      verb: slip\n    do:\n      - location:\n          to: cellar\n          teleport: true\n";
    let text = text.replacen("\n---\n", &format!("{rules}---\n"), 1);
    assert!(text.contains("id: slip"), "{text}");
    fs::write(&path, text).unwrap();
    edit_save(&pack, |_| {});
    table::listen(world, &pack).await;
    table::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[then(expr = "the PC reaches cellar with no exit")]
async fn the_pc_reaches_the_cellar(world: &mut World) {
    let (_mark, added) = table::player_send(world, "slip", "cellar").await;
    assert_eq!(
        place_of(&world_of(table::pack(world)), "rowan"),
        "cellar",
        "{added}"
    );
}

#[then(expr = "the world is at tick 1300 and rowan is in mira_rooms")]
fn rowan_is_in_mira_rooms_at_tick_1300(world: &mut World) {
    let now = world_of(table::pack(world));
    assert_eq!(now.world_time, EVENING + 10);
    assert_eq!(place_of(&now, "rowan"), "mira_rooms");
}

#[then(expr = "rowan's line shows {string}")]
async fn rowans_line_shows(world: &mut World, text: String) {
    let needle = format!("\"place\":\"{text}\"");
    let added = table::wait_player_from(world, world.mark, &needle).await;
    assert!(added.contains(&needle), "{added}");
}

/// Add nell, a PC with wren's meters and nothing else, at `place`.
fn add_nell(now: &mut ecs::World, place: &str) {
    let nell = ActorId::from("nell");
    let wren = "wren".into();
    let meters: Vec<_> = (now.actors().get::<Meter>(&wren).iter())
        .map(|(key, meter)| (key.to_string(), meter.clone()))
        .collect();
    let mut actors = now.actors_mut();
    actors.add_actor(&nell, component::ControlType::Pc, place);
    for (key, meter) in meters {
        actors
            .get_mut::<Meter>(&nell)
            .set(&key, Some(meter))
            .unwrap();
    }
}

#[given(expr = "rounds are 5 ticks while seated and the stairs exit costs 2 ticks")]
async fn seated_rounds_are_five_ticks(world: &mut World) {
    let pack = copy_pack("docks", "exact-tick");
    edit_save(&pack, |now| {
        give_key(now);
        now.world_time = EVENING;
    });
    table::listen(world, &pack).await;
    table::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs in the round starting at tick 1290")]
async fn rowan_travels_in_the_evening_round(world: &mut World) {
    let (_mark, added) = table::player_send(world, "travel stairs", "mira_rooms").await;
    world.player_added = added;
}

#[then(expr = "the round still ends at tick 1295")]
fn the_round_still_ends_at_tick_1295(world: &mut World) {
    assert_eq!(world_of(table::pack(world)).world_time, EVENING + 5);
}

#[then(expr = "a move from rowan is accepted from the round starting at tick 1295")]
async fn the_next_round_accepts_a_move(world: &mut World) {
    let (_mark, added) = table::player_send(world, "wait", "\"kind\":").await;
    assert!(!added.contains("BUSY"), "{added}");
    let now = world_of(table::pack(world));
    assert_eq!(now.world_time, EVENING + 10);
    assert!(!is_busy(&now, "rowan"));
}

#[then(expr = "a seated round advances world_time by 5 and an empty one by 60")]
fn rounds_never_shorten(_world: &mut World) {
    let seated = copy_pack("docks", "seated-length");
    let mut seated_session = spookery_engine::testing::resume(&seated);
    let one_human = spookery_engine::Presence {
        players: 1,
        actors_present: Vec::new(),
    };
    let wall = seated_session.world().last_round_at.clone();
    seated_session.play(&one_human, &wall).unwrap();
    drop(seated_session);
    let idle = copy_pack("docks", "idle-length");
    spookery_engine::play_pack_once(&idle).unwrap();
    let start = world_of(&copy_pack("docks", "start-time")).world_time;
    assert_eq!(world_of(&seated).world_time - start, 5);
    assert_eq!(world_of(&idle).world_time - start, 60);
}

#[given(expr = "rowan and nell are seated and both busy, with no GM")]
async fn both_busy_without_a_gm(world: &mut World) {
    let pack = copy_pack("docks", "fast-forward");
    add_toll(&pack);
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "rowan", EVENING + 10, "docks_shop.stairs");
    });
    fs::write(
        pack.join("cast/pc-nell.md"),
        "---\nschema: cast\nid: nell\ncontrol_type: pc_template\nname: Nell\nidle:\n  eligible: false\n---\nNell.\n",
    )
    .unwrap();
    edit_save(&pack, |now| {
        add_nell(now, "docks_shop");
        set_busy(now, "nell", EVENING + 5, "docks_shop.docks_street");
    });
    table::listen(world, &pack).await;
    table::seat_rowan(world, &pack).await;
    table::seat_actor(world, &pack, "nell").await;
    world.pack = Some(pack);
}

#[then(
    expr = "the table plays rounds to tick 1295 then tick 1300 without waiting on the wall clock"
)]
async fn the_table_plays_rounds_through_both_arrivals(world: &mut World) {
    let (mark, _added) = table::player_send(world, "wait", "BUSY").await;
    let rounds = table::wait_rounds_to_view(world, mark).await;
    assert_eq!(rounds, [EVENING + 5, EVENING + 10]);
    let now = world_of(table::pack(world));
    assert_eq!(place_of(&now, "nell"), "docks_street");
    assert_eq!(place_of(&now, "rowan"), "mira_rooms");
}

#[given(expr = "a GM is seated and both PCs are busy")]
async fn a_gm_and_two_busy_pcs(world: &mut World) {
    both_busy_without_a_gm(world).await;
    table::seat_gm(world).await;
}

#[then(expr = "nothing advances until the GM commits")]
async fn nothing_advances_until_commit(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let before = world_of(&pack).world_time;
    let (_mark, added) = table::player_send(world, "wait", "BUSY").await;
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(world_of(&pack).world_time, before);
    let _ = table::gm_tool_call(world, 9, "commit", r#"{"include_round":true}"#).await;
    assert!(world_of(&pack).world_time > before);
}

#[given(expr = "busy_until is 200 rounds away and max_fastforward_rounds is 12")]
async fn a_long_journey_and_the_cap(world: &mut World) {
    let pack = copy_pack("docks", "cap");
    edit_save(&pack, |now| {
        now.world_time = EVENING;
        set_busy(now, "rowan", EVENING + 200 * 5, "docks_shop.stairs");
    });
    table::listen(world, &pack).await;
    table::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[then(expr = "12 rounds run and the rest continues on the next action")]
async fn twelve_rounds_then_more(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let (mark, added) = table::player_send(world, "wait", "BUSY").await;
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(
        table::wait_rounds_to_view(world, mark).await.len(),
        12,
        "first burst"
    );
    let mid = world_of(&pack).world_time;
    assert_eq!(mid, EVENING + 12 * 5);
    assert!(is_busy(&world_of(&pack), "rowan"));
    let (mark, added) = table::player_send(world, "wait", "BUSY").await;
    assert!(added.contains("BUSY"), "{added}");
    assert_eq!(
        table::wait_rounds_to_view(world, mark).await.len(),
        12,
        "second burst"
    );
    assert_eq!(world_of(&pack).world_time, mid + 12 * 5);
}

#[given(expr = "rowan visited the shop and mira_rooms in an earlier session")]
fn rowan_visited_both_rooms(world: &mut World) {
    let pack = copy_pack("docks", "map");
    add_trapdoor(&pack);
    edit_save(&pack, |now| {
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
async fn play_joins_again(world: &mut World) {
    play_as_pc(world, "rowan", &[]).await;
}

/// The map is the table's: each place rowan's save says they visited, with the exits
/// rowan saw there last.
#[then(expr = "play's map lists the shop and mira_rooms and not the trapdoor")]
async fn plays_map_lists_both_rooms(world: &mut World) {
    let mark = capture_play_stdout(world).len();
    write_play(world, "map");
    wait_play_until(
        world,
        |stdout| stdout[mark..].contains("Mira's Rooms\n  ") && stdout.ends_with('\n'),
        "the map",
    )
    .await;
    let after = &capture_play_stdout(world)[mark..];
    let map = &after[after.find("Docks Shop\n  ").unwrap_or_default()..];
    assert!(map.starts_with("Docks Shop\n  "), "{map}");
    assert!(map.contains("Mira's Rooms\n  "), "{map}");
    assert!(!map.contains("trapdoor"), "{map}");
}

/// The way onto the lane: Briar steps aside, and the chock holds the cart's wheel.
const LANE_SCRIPT: &[&str] = &[
    "ask briar",
    "travel wheel_shed",
    "take chock",
    "travel gate_yard",
    "chock",
    "travel mill_lane",
];

#[when(expr = "the lane script is played with the stub GM")]
async fn the_lane_script_is_played_with_the_stub_gm(world: &mut World) {
    let pack = copy_pack("docks", "lane-script");
    table::listen(world, &pack).await;
    table::seat_gm(world).await;
    table::seat_actor(world, &pack, "harbormouse").await;
    world.pack = Some(pack.clone());
    let (lane, way) = LANE_SCRIPT.split_last().unwrap();
    for line in way {
        commit_line(world, line).await;
    }
    world.start_ticks = world_of(&pack).world_time;
    commit_line(world, lane).await;
}

#[then(expr = "Harbormouse reaches the lane at the exit's cost in ticks and hears the ending")]
async fn harbormouse_reaches_the_lane_and_hears_the_ending(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let session = spookery_engine::Session::peek(&pack).unwrap();
    let map = session.map(&ActorId::from("harbormouse")).unwrap();
    let lane = (map.iter().flat_map(|place| &place.exits)).find(|exit| exit.exit_id == "mill_lane");
    let lane_ticks = lane.unwrap().ticks;
    assert!(lane_ticks > 0);
    let now = world_of(&pack);
    assert_eq!(place_of(&now, "harbormouse"), "mill_lane");
    assert_eq!(now.world_time - world.start_ticks, lane_ticks);
    let flags = now
        .entities()
        .get::<component::Flag>(&spookery_engine::entity::Id::Global);
    assert!(flags.contains("finished"), "finished is not set");
    table::wait_player_from(
        world,
        0,
        "The cart rolls out onto the lane. That is the end.",
    )
    .await;
}

/// The player sends `line`, then the GM commits its round.
async fn commit_line(world: &mut World, line: &str) {
    let mark = table::player_write(world, line);
    table::wait_player_from(world, mark, "\"kind\":\"queued\"").await;
    table::gm_tool_call(world, 2, "commit", r#"{"include_round":true}"#).await;
}
