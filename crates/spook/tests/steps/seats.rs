use std::fs;

use cucumber::{given, then, when};
use spookery_engine::testing::journal::list_tick_keys;

use crate::World;

use super::table;

#[when(expr = "a second player connects")]
fn a_second_player_connects(world: &mut World) {
    world.scratch = table::second_hello(world, "player:pc_rowan");
}

#[when(expr = "a second GM connects over HTTP")]
fn a_second_gm_connects_over_http(world: &mut World) {
    let http = table::live_mut(world).http_port;
    world.scratch = super::mcp::http_post(http, "127.0.0.1", None);
}

#[when(expr = "a socket sends gm on the game port")]
fn a_socket_sends_gm_on_the_game_port(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    world.scratch = table::second_hello(world, "gm");
}

#[when(regex = r"^a socket sends (an empty hello|player:)$")]
fn a_socket_sends_hello(world: &mut World, which: String) {
    if world.live.is_none() {
        listen_pack(world);
    }
    world.scratch = table::second_hello(world, if which == "player:" { "player:" } else { "" });
}

#[then(expr = "the hello is refused with {string} and the missing list is {string}")]
fn the_hello_is_refused_with_missing(world: &mut World, code: String, missing: String) {
    the_hello_is_refused_with(world, code);
    let frame: serde_json::Value = serde_json::from_str(world.scratch.trim()).unwrap();
    let raw = frame["missing"].as_str().unwrap_or("");
    assert!(serde_json::from_str::<Vec<String>>(raw).is_ok(), "{raw}");
    assert_eq!(raw, missing);
}

#[then(expr = "the hello reply does not contain {string}")]
fn the_hello_reply_does_not_contain(world: &mut World, needle: String) {
    assert!(!world.scratch.contains(&needle), "{}", world.scratch);
}

#[when(expr = "play joins that one-player table with no --as")]
fn play_joins_that_one_player_table_with_no_as(world: &mut World) {
    the_player_leaves(world);
    let pack = crate::support::pack_arg(table::pack(world));
    let addr = table::game_addr(world);
    world.play = Some(crate::support::spawn(&["play", &pack, "--addr", &addr]));
    super::common::wait_play(world, "You are in");
}

#[then(expr = "save.yaml is unchanged since the table opened")]
fn now_unchanged_since_open(world: &mut World) {
    let opened = table::live_mut(world).opened_now.clone();
    let now = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    assert_eq!(now, opened);
}

#[then(expr = "humans is {int}")]
fn humans_is(world: &mut World, count: u32) {
    assert_eq!(table::live_presence(world).0, count);
}

#[then(expr = "the present actor is pc_rowan")]
fn the_present_actor_is_pc_rowan(world: &mut World) {
    assert_eq!(table::live_presence(world).1, vec!["pc_rowan".to_string()]);
}

#[then(expr = "state_rev is 0")]
fn state_rev_is_0(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).state_rev, 0);
}

#[when(expr = "the state directory is remembered")]
fn the_state_directory_is_remembered(world: &mut World) {
    world.before = table::files(&table::pack(world).join("state"));
}

#[then(expr = "the state directory is unchanged")]
fn the_state_directory_is_unchanged(world: &mut World) {
    assert_eq!(
        table::files(&table::pack(world).join("state")),
        world.before
    );
}

#[when(expr = "a player sits")]
fn a_player_sits(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_rowan");
}

#[when(expr = "the player leaves")]
fn the_player_leaves(world: &mut World) {
    table::close_actor(world, "pc_rowan");
    let pack = table::pack(world).to_path_buf();
    table::wait_humans(world, &pack, 0);
}

#[when(expr = "the host restarts")]
fn the_host_restarts(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::stop_server(world);
    table::listen(world, &pack);
}

#[then(expr = "nobody is present")]
fn nobody_is_present(world: &mut World) {
    let (humans, actors) = table::live_presence(world);
    assert_eq!(humans, 0);
    assert!(actors.is_empty(), "{actors:?}");
}

#[then(expr = "the session ids differ")]
fn the_session_ids_differ(world: &mut World) {
    let ids: Vec<&str> = world.scratch.lines().collect();
    let [rowan, wren, again] = ids[..] else {
        panic!("expected three session ids, got {:?}", world.scratch);
    };
    assert_ne!(rowan, wren);
    assert_ne!(again, wren);
    assert_ne!(again, rowan);
}

const DOCKS_PCS: [&str; 3] = ["pc_harbormouse", "pc_rowan", "pc_wren"];
const HIDDEN_FACT: &str = "fact_wren_channel";

#[when(expr = "three players and a GM sit")]
fn three_players_and_a_gm_sit(world: &mut World) {
    listen_pack(world);
    table::seat_gm(world);
    let pack = table::pack(world).to_path_buf();
    seat_three(world, &pack);
}

#[given(expr = "a docks table whose pack defines four PCs")]
fn a_docks_table_whose_pack_defines_four_pcs(world: &mut World) {
    let pack = crate::support::temp_pack("four-pcs");
    let cast = "\
---
schema: cast
id: pc_lark
kind: pc_template
display_name: Lark
facts: []
tick:
  eligible: false
---
Lark watches the street from a dry doorway.
";
    fs::write(pack.join("cast/pc-lark.md"), cast).unwrap_or_else(|err| panic!("cast: {err}"));
    crate::support::edit_save(&pack, |now| {
        let mut actor = now
            .actors
            .get("pc_rowan")
            .unwrap_or_else(|| panic!("pc_rowan"))
            .clone();
        *actor.place_mut() = "docks_street".to_string();
        now.actors.insert("pc_lark".to_string(), actor);
    });
    spookery_engine::World::open(&pack).unwrap_or_else(|err| panic!("four-pc pack: {err}"));
    world.pack = Some(pack);
}

#[when(expr = "three players sit")]
fn three_players_sit(world: &mut World) {
    listen_pack(world);
    let pack = table::pack(world).to_path_buf();
    seat_three(world, &pack);
}

#[when(expr = "a fourth player connects")]
fn a_fourth_player_connects(world: &mut World) {
    world.scratch = table::second_hello(world, "player:pc_lark");
}

#[then(expr = "the hello is refused with {string}")]
fn the_hello_is_refused_with(world: &mut World, code: String) {
    let tcp = format!("\"code\":\"{code}\"");
    let http = format!("[{code}]");
    assert!(
        world.scratch.contains(&tcp) || world.scratch.contains(&http),
        "hello reply:\n{}",
        world.scratch
    );
}

#[then(expr = "the present actors are the three docks PCs")]
fn the_present_actors_are_the_three_docks_pcs(world: &mut World) {
    assert_eq!(
        table::live_presence(world).1,
        DOCKS_PCS.map(str::to_string).to_vec()
    );
}

#[given(expr = "a one-player docks table")]
fn a_one_player_docks_table(world: &mut World) {
    let pack = crate::support::temp_pack("one-pc");
    fs::remove_file(pack.join("cast/pc-harbormouse.md"))
        .unwrap_or_else(|err| panic!("remove harbormouse: {err}"));
    fs::remove_file(pack.join("cast/pc-wren.md"))
        .unwrap_or_else(|err| panic!("remove wren: {err}"));
    crate::support::edit_save(&pack, |now| {
        now.actors.remove("pc_harbormouse");
        now.actors.remove("pc_wren");
    });
    spookery_engine::World::open(&pack).unwrap_or_else(|err| panic!("one-pc pack: {err}"));
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_rowan");
    world.pack = Some(pack);
}

#[given(expr = "a docks table with three seated players")]
fn a_docks_table_with_three_seated_players(world: &mut World) {
    let pack = crate::support::temp_pack("three-players");
    table::listen(world, &pack);
    seat_three(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "the first player drops")]
fn the_first_player_drops(world: &mut World) {
    table::close_actor(world, "pc_rowan");
    let pack = table::pack(world).to_path_buf();
    table::wait_humans(world, &pack, 2);
}

#[then(expr = "the present actors are pc_harbormouse and pc_wren")]
fn the_present_actors_are_harbormouse_and_wren(world: &mut World) {
    assert_eq!(
        table::live_presence(world).1,
        vec!["pc_harbormouse".to_string(), "pc_wren".to_string()]
    );
}

#[given(expr = "a docks table where one PC knows a hidden fact")]
fn a_docks_table_where_one_pc_knows_a_hidden_fact(world: &mut World) {
    let pack = crate::support::temp_pack("hidden-fact");
    let path = pack.join("cast/pc-wren.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read cast: {err}"));
    let text = text.replace(
        "facts: []\n",
        "facts:\n  - id: fact_wren_channel\n    text: The tide chart marks a private channel.\n    hidden: true\n",
    );
    fs::write(&path, text).unwrap_or_else(|err| panic!("write cast: {err}"));
    crate::support::edit_save(&pack, |now| {
        now.actors
            .get_mut("pc_wren")
            .unwrap_or_else(|| panic!("pc_wren"))
            .known_facts_mut()
            .push(HIDDEN_FACT.to_string());
    });
    spookery_engine::World::open(&pack).unwrap_or_else(|err| panic!("hidden fact pack: {err}"));
    world.pack = Some(pack);
}

#[then(expr = "only that PC's perceive shows the hidden fact")]
fn only_that_pcs_perceive_shows_the_hidden_fact(world: &mut World) {
    let wren = table::actor_text(world, "pc_wren");
    assert!(wren.contains(HIDDEN_FACT), "{wren}");
    for id in ["pc_rowan", "pc_harbormouse"] {
        let text = table::actor_text(world, id);
        assert!(!text.contains(HIDDEN_FACT), "{id}\n{text}");
    }
}

#[given(expr = "a docks table with a GM and two seated players")]
fn a_docks_table_with_a_gm_and_two_seated_players(world: &mut World) {
    let pack = crate::support::temp_pack("two-queue");
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "pc_rowan");
    table::seat_actor(world, &pack, "pc_harbormouse");
    world.pack = Some(pack);
}

#[given(expr = "a docks table with two seated players")]
fn a_docks_table_with_two_seated_players(world: &mut World) {
    let pack = crate::support::temp_pack("two-live");
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_rowan");
    table::seat_actor(world, &pack, "pc_harbormouse");
    world.pack = Some(pack);
}

#[when(expr = "both players wait")]
fn both_players_wait(world: &mut World) {
    let ids: Vec<String> = table::live_mut(world)
        .players
        .iter()
        .map(|player| player.actor_id.clone())
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    for id in ids {
        world.player_added = table::actor_send(world, &id, "wait", "\"kind\":");
    }
}

#[then(expr = "both lines are queued for the GM")]
fn both_lines_are_queued_for_the_gm(world: &mut World) {
    let queued = |text: &str, id: &str| {
        text.lines().any(|line| {
            line.contains("\"kind\":\"queued\"")
                && line.contains(&format!("\"actor_id\":\"{id}\""))
                && line.contains("\"move\":\"wait\"")
        })
    };
    table::wait_gm_until(
        world,
        "queued waits for pc_rowan and pc_harbormouse",
        |text| queued(text, "pc_rowan") && queued(text, "pc_harbormouse"),
    );
}

#[then(expr = "session status lists both queued lines")]
fn session_status_lists_both_queued_lines(world: &mut World) {
    let text = table::gm_call(world, 4, "session_status", "{}");
    for id in ["pc_rowan", "pc_harbormouse"] {
        assert!(text.contains(id), "{text}");
    }
    assert!(
        text.matches("wait").count() >= 2,
        "queue did not list both waits\n{text}"
    );
}

#[then(expr = "the tick log records both waits")]
fn the_tick_log_records_both_waits(world: &mut World) {
    let actors = table::list_player_waits(table::pack(world), "tick-0001");
    assert_eq!(
        actors,
        vec!["pc_rowan".to_string(), "pc_harbormouse".to_string()]
    );
}

#[then(expr = "each wait committed its own tick")]
fn each_wait_committed_its_own_tick(world: &mut World) {
    let pack = table::pack(world);
    table::wait_world_time(pack, 1090);
    assert_eq!(table::now_of(pack).tick_index, 2);
    assert_eq!(
        list_tick_keys(pack),
        vec!["tick-0001".to_string(), "tick-0002".to_string()]
    );
    assert_eq!(
        table::list_player_waits(pack, "tick-0001"),
        vec!["pc_rowan".to_string()]
    );
    assert_eq!(
        table::list_player_waits(pack, "tick-0002"),
        vec!["pc_harbormouse".to_string()]
    );
}

#[given(expr = "a docks table with two PCs in the shop")]
fn a_docks_table_with_two_pcs_in_the_shop(world: &mut World) {
    let pack = crate::support::temp_pack("two-shop");
    crate::support::edit_save(&pack, |now| {
        *now.actors
            .get_mut("pc_harbormouse")
            .unwrap_or_else(|| panic!("pc_harbormouse"))
            .place_mut() = "docks_shop".to_string();
    });
    table::listen(world, &pack);
    seat_three(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "the shop player speaks to the place")]
fn the_shop_player_speaks_to_the_place(world: &mut World) {
    let _ = table::actor_send(
        world,
        "pc_rowan",
        "say place the blue rope is tied",
        "the blue rope is tied",
    );
    let start = std::time::Instant::now();
    loop {
        if table::actor_text(world, "pc_harbormouse").contains("the blue rope is tied") {
            return;
        }
        if start.elapsed() > std::time::Duration::from_secs(5) {
            panic!(
                "harbormouse missed the place say\n{}",
                table::actor_text(world, "pc_harbormouse")
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[then(expr = "only the shop players receive the say")]
fn only_the_shop_players_receive_the_say(world: &mut World) {
    assert!(table::actor_text(world, "pc_rowan").contains("the blue rope is tied"));
    assert!(table::actor_text(world, "pc_harbormouse").contains("the blue rope is tied"));
    std::thread::sleep(std::time::Duration::from_millis(200));
    let wren = table::actor_text(world, "pc_wren");
    assert!(!wren.contains("the blue rope is tied"), "{wren}");
}

#[when(expr = "three play clients sit on a free port")]
fn three_play_clients_sit_on_a_free_port(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    let addr = table::game_addr(world);
    let pack_arg = crate::support::pack_arg(&pack);
    for id in DOCKS_PCS {
        let as_arg = format!("player:{id}");
        world.plays.push(crate::support::spawn(&[
            "play", &pack_arg, "--addr", &addr, "--as", &as_arg,
        ]));
    }
}

#[then(expr = "each play client shows its first perceive")]
fn each_play_client_shows_its_first_perceive(world: &mut World) {
    assert_eq!(world.plays.len(), 3);
    for play in &mut world.plays {
        wait_play_proc(play, "You are in");
    }
}

fn listen_pack(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
}

fn seat_three(world: &mut World, pack: &std::path::Path) {
    for id in ["pc_rowan", "pc_harbormouse", "pc_wren"] {
        table::seat_actor(world, pack, id);
    }
}

fn wait_play_proc(play: &mut crate::support::Proc, needle: &str) {
    let start = std::time::Instant::now();
    loop {
        let stdout = crate::support::text_of(&play.stdout);
        if stdout.contains(needle) {
            return;
        }
        if let Some(status) = play
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            let stderr = crate::support::text_of(&play.stderr);
            panic!("play exited {status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        if start.elapsed() > std::time::Duration::from_secs(8) {
            let stderr = crate::support::text_of(&play.stderr);
            panic!("missing {needle}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[then(expr = "the tick now_hash matches the save")]
fn the_tick_now_hash_matches_the_save(world: &mut World) {
    let pack = table::pack(world);
    let tick = spookery_engine::testing::journal::read_record_line(pack, "tick-0001")
        .unwrap_or_else(|err| panic!("read tick: {}", err.detail));
    let value: serde_json::Value =
        serde_json::from_str(&tick).unwrap_or_else(|err| panic!("{err}"));
    let expected = value["now_hash"]
        .as_str()
        .unwrap_or_else(|| panic!("now_hash\n{tick}"));
    let report = spookery_engine::testing::from_journal(pack, None, None)
        .unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(
        spookery_engine::testing::journal::hash_now(&report.now_yaml),
        expected
    );
}

#[when(expr = "two players sit, one drops, and that PC sits again")]
fn two_players_sit_and_one_sits_again(world: &mut World) {
    world.scratch = seat_drop_and_reseat().join("\n");
}

/// Seat Rowan and Wren, drop Rowan, seat Rowan again. Returns the three session ids in order.
fn seat_drop_and_reseat() -> [String; 3] {
    let pack = crate::support::temp_pack("session-ids");
    let mut shard =
        spookery_engine::World::open(&pack).unwrap_or_else(|err| panic!("World::open: {err}"));
    let rowan = shard.seat_player("pc_rowan").unwrap();
    let wren = shard.seat_player("pc_wren").unwrap();
    assert!(shard.release_player_seat(&rowan, "pc_rowan").unwrap());
    let again = shard.seat_player("pc_rowan").unwrap();
    let _ = fs::remove_dir_all(&pack);
    [rowan, wren, again]
}
