use spookery_engine::testing::state_of;
use std::fs;

use cucumber::{given, then, when};
use spookery_engine::testing::journal::list_round_keys;

use crate::World;

use super::table;

#[when(expr = "a second player connects")]
fn a_second_player_connects(world: &mut World) {
    world.scratch = table::claim(world, "pc_rowan");
}

#[when(expr = "a second GM connects on the game port")]
fn a_second_gm_connects_on_the_game_port(world: &mut World) {
    let seat = table::Seat::open(table::live_mut(world).game_port, "gm");
    world.scratch = table::wait_suffix(&seat.text, 0, "\"kind\":\"refused\"");
    seat.wait_closed();
}

#[when(expr = "an unclaimed player waits")]
fn an_unclaimed_player_waits(world: &mut World) {
    if world.live.is_none() {
        listen_pack(world);
    }
    let seat = table::Seat::open(table::live_mut(world).game_port, "player");
    let id = seat.send("wait", serde_json::json!({}));
    world.scratch = seat.wait_reply(&id);
}

#[when(expr = "a player claims an empty id")]
fn a_player_claims_an_empty_id(world: &mut World) {
    world.scratch = table::claim(world, "");
}

#[when(expr = "a player claims {word}")]
fn a_player_claims(world: &mut World, actor: String) {
    listen_pack(world);
    world.scratch = table::claim(world, &actor);
}

#[then(expr = "the hello reply does not contain {string}")]
fn the_hello_reply_does_not_contain(world: &mut World, needle: String) {
    assert!(!world.scratch.contains(&needle), "{}", world.scratch);
}

#[when(expr = "the operator runs play on that table")]
fn the_operator_runs_play_on_that_table(world: &mut World) {
    let addr = table::game_addr(world);
    table::run(world, &["play", "--addr", &addr]);
}

#[then(expr = "save.yaml is unchanged since the table opened")]
fn save_unchanged_since_open(world: &mut World) {
    let opened = table::live_mut(world).opened_save.clone();
    let now = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    assert_eq!(now, opened);
}

#[then(expr = "players is {int}")]
fn players_is(world: &mut World, count: u32) {
    assert_eq!(table::read_presence(world).0, count);
}

#[then(expr = "the present actor is pc_rowan")]
fn the_present_actor_is_pc_rowan(world: &mut World) {
    assert_eq!(table::read_presence(world).1, vec!["pc_rowan".to_string()]);
}

#[then(expr = "state_rev is 0")]
fn state_rev_is_0(world: &mut World) {
    assert_eq!(state_of(table::pack(world)).state_rev, 0);
}

#[when(expr = "the state directory is remembered")]
fn the_state_directory_is_remembered(world: &mut World) {
    world.before = table::read_files(&table::pack(world).join("state"));
}

#[then(expr = "the state directory is unchanged")]
fn the_state_directory_is_unchanged(world: &mut World) {
    assert_eq!(
        table::read_files(&table::pack(world).join("state")),
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
    table::wait_players(world, 0);
}

#[when(expr = "the host restarts")]
fn the_host_restarts(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::stop_host(world);
    table::listen(world, &pack);
}

#[then(expr = "nobody is present")]
fn nobody_is_present(world: &mut World) {
    let (players, actors) = table::read_presence(world);
    assert_eq!(players, 0);
    assert!(actors.is_empty(), "{actors:?}");
}

const DOCKS_PCS: [&str; 3] = ["pc_harbormouse", "pc_rowan", "pc_wren"];
const HIDDEN_FACT: &str = "fact_wren_channel";
const HIDDEN_TEXT: &str = "The tide chart marks a private channel.";

#[when(expr = "three players and a GM sit")]
fn three_players_and_a_gm_sit(world: &mut World) {
    listen_pack(world);
    table::seat_gm(world);
    let pack = table::pack(world).to_path_buf();
    seat_three(world, &pack);
}

#[given(expr = "a docks table whose pack defines four PCs")]
fn a_docks_table_whose_pack_defines_four_pcs(world: &mut World) {
    let pack = crate::support::copy_docks("four-pcs");
    let cast = "\
---
schema: cast
id: pc_lark
kind: pc_template
display_name: Lark
idle:
  eligible: false
---
Lark watches the street from a dry doorway.
";
    fs::write(pack.join("cast/pc-lark.md"), cast).unwrap_or_else(|err| panic!("cast: {err}"));
    crate::support::edit_save(&pack, |now| {
        spookery_engine::testing::copy_actor(now, "pc_wren", "pc_lark");
        let lark = now.actors.get_mut("pc_lark").unwrap().frontier_mut();
        lark.place = "docks_street".to_string();
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
        table::read_presence(world).1,
        DOCKS_PCS.map(str::to_string).to_vec()
    );
}

#[given(expr = "a one-player docks table")]
fn a_one_player_docks_table(world: &mut World) {
    let pack = crate::support::copy_docks("one-pc");
    fs::remove_file(pack.join("cast/pc-harbormouse.md"))
        .unwrap_or_else(|err| panic!("remove harbormouse: {err}"));
    fs::remove_file(pack.join("cast/pc-wren.md"))
        .unwrap_or_else(|err| panic!("remove wren: {err}"));
    crate::support::edit_save(&pack, |now| {
        spookery_engine::testing::remove_actor(now, "pc_harbormouse");
        spookery_engine::testing::remove_actor(now, "pc_wren");
    });
    spookery_engine::World::open(&pack).unwrap_or_else(|err| panic!("one-pc pack: {err}"));
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_rowan");
    world.pack = Some(pack);
}

#[given(expr = "a docks table with three seated players")]
fn a_docks_table_with_three_seated_players(world: &mut World) {
    let pack = crate::support::copy_docks("three-players");
    table::listen(world, &pack);
    seat_three(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "the first player drops")]
fn the_first_player_drops(world: &mut World) {
    table::close_actor(world, "pc_rowan");
    table::wait_players(world, 2);
}

#[then(expr = "the present actors are pc_harbormouse and pc_wren")]
fn the_present_actors_are_harbormouse_and_wren(world: &mut World) {
    assert_eq!(
        table::read_presence(world).1,
        vec!["pc_harbormouse".to_string(), "pc_wren".to_string()]
    );
}

#[given(expr = "a docks table where one PC knows a hidden fact")]
fn a_docks_table_where_one_pc_knows_a_hidden_fact(world: &mut World) {
    let pack = crate::support::copy_docks("hidden-fact");
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read shop: {err}"));
    let from = "    text: Oil is on the shelf.\n";
    assert!(text.contains(from), "{text}");
    let text = text.replace(
        from,
        &format!("{from}  - id: {HIDDEN_FACT}\n    text: {HIDDEN_TEXT}\n    hidden: true\n"),
    );
    fs::write(&path, text).unwrap_or_else(|err| panic!("write shop: {err}"));
    crate::support::edit_save(&pack, |now| {
        now.actors
            .get_mut("pc_wren")
            .unwrap_or_else(|| panic!("pc_wren"))
            .frontier_mut()
            .known_facts
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

#[when(expr = "that PC says the hidden fact and another PC waits")]
fn that_pc_says_the_hidden_fact(world: &mut World) {
    let marks = ["pc_rowan", "pc_harbormouse"].map(|id| (id, table::actor_text(world, id).len()));
    let say = format!("say {HIDDEN_TEXT}");
    table::actor_send(world, "pc_wren", &say, "\"kind\":\"say\"");
    table::actor_send(world, "pc_rowan", "wait", "\"kind\":\"view\"");
    for (id, mark) in marks {
        let heard = table::wait_actor_from(world, id, mark, "\"kind\":\"view\"");
        assert!(heard.contains(HIDDEN_TEXT), "{id} did not hear it\n{heard}");
    }
}

#[given(expr = "a docks table with a GM and two seated players")]
fn a_docks_table_with_a_gm_and_two_seated_players(world: &mut World) {
    let pack = crate::support::copy_docks("two-queue");
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_actor(world, &pack, "pc_rowan");
    table::seat_actor(world, &pack, "pc_harbormouse");
    world.pack = Some(pack);
}

#[given(expr = "a docks table with two seated players")]
fn a_docks_table_with_two_seated_players(world: &mut World) {
    let pack = crate::support::copy_docks("two-live");
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
                && line.contains(&format!("\"actor\":\"{id}\""))
                && line.contains("\"move\":\"wait\"")
        })
    };
    table::wait_gm_until(
        world,
        "queued waits for pc_rowan and pc_harbormouse",
        |text| queued(text, "pc_rowan") && queued(text, "pc_harbormouse"),
    );
}

#[then(expr = "table status lists both queued lines")]
fn table_status_lists_both_queued_lines(world: &mut World) {
    let text = table::gm_tool_call(world, 4, "status", "{}");
    for id in ["pc_rowan", "pc_harbormouse"] {
        assert!(text.contains(id), "{text}");
    }
    assert!(
        text.matches("wait").count() >= 2,
        "queue did not list both waits\n{text}"
    );
}

#[then(expr = "the round log records both waits")]
fn the_round_log_records_both_waits(world: &mut World) {
    let actors = table::list_player_waits(table::pack(world), "round-0001");
    assert_eq!(
        actors,
        vec!["pc_rowan".to_string(), "pc_harbormouse".to_string()]
    );
}

#[then(expr = "each wait committed its own round")]
fn each_wait_committed_its_own_round(world: &mut World) {
    table::wait_round(world, 1090);
    let pack = table::pack(world);
    assert_eq!(state_of(pack).round_index, 2);
    assert_eq!(
        list_round_keys(pack),
        vec!["round-0001".to_string(), "round-0002".to_string()]
    );
    assert_eq!(
        table::list_player_waits(pack, "round-0001"),
        vec!["pc_rowan".to_string()]
    );
    assert_eq!(
        table::list_player_waits(pack, "round-0002"),
        vec!["pc_harbormouse".to_string()]
    );
}

#[given(expr = "a docks table with two PCs in the shop")]
fn a_docks_table_with_two_pcs_in_the_shop(world: &mut World) {
    let pack = crate::support::copy_docks("two-shop");
    crate::support::edit_save(&pack, |now| {
        now.actors
            .get_mut("pc_harbormouse")
            .unwrap_or_else(|| panic!("pc_harbormouse"))
            .frontier_mut()
            .place = "docks_shop".to_string();
    });
    table::listen(world, &pack);
    seat_three(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "the shop player speaks to the place")]
fn the_shop_player_speaks_to_the_place(world: &mut World) {
    let rowan = &table::player_client(world, "pc_rowan").seat;
    let say = serde_json::json!({
        "address": "place",
        "place": "docks_shop",
        "text": "the blue rope is tied",
    });
    let id = rowan.send("say", say);
    rowan.wait_reply(&id);
    table::poll_until("harbormouse to hear the place say", || {
        let heard = table::actor_text(world, "pc_harbormouse");
        if heard.contains("the blue rope is tied") {
            Ok(())
        } else {
            Err(heard)
        }
    });
}

#[then(expr = "only the shop players receive the say")]
fn only_the_shop_players_receive_the_say(world: &mut World) {
    assert!(table::actor_text(world, "pc_rowan").contains("the blue rope is tied"));
    assert!(table::actor_text(world, "pc_harbormouse").contains("the blue rope is tied"));
    // The table answers in order, so Wren's seat would have the say before this reply.
    let wren = &table::player_client(world, "pc_wren").seat;
    let id = wren.send("look", serde_json::json!({}));
    wren.wait_reply(&id);
    let wren = table::actor_text(world, "pc_wren");
    assert!(!wren.contains("the blue rope is tied"), "{wren}");
}

#[when(expr = "three play clients sit on a free port")]
fn three_play_clients_sit_on_a_free_port(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    let addr = table::game_addr(world);
    for id in DOCKS_PCS {
        world.plays.push(super::common::spawn_play(&addr, id, &[]));
    }
}

#[then(expr = "each play client shows its first perceive")]
fn each_play_client_shows_its_first_perceive(world: &mut World) {
    assert_eq!(world.plays.len(), 3);
    for play in &mut world.plays {
        let you_are_in = |stdout: &str| stdout.contains("You are in");
        super::common::wait_proc_until(play, you_are_in, "You are in");
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

#[then(expr = "the round state_hash matches the save")]
fn the_round_state_hash_matches_the_save(world: &mut World) {
    let pack = table::pack(world);
    let round = spookery_engine::testing::journal::read_record_line(pack, "round-0001")
        .unwrap_or_else(|err| panic!("read round: {}", err.detail));
    let value: serde_json::Value =
        serde_json::from_str(&round).unwrap_or_else(|err| panic!("{err}"));
    let expected = value["state_hash"]
        .as_str()
        .unwrap_or_else(|| panic!("state_hash\n{round}"));
    let report = spookery_engine::testing::replay_pack(pack).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(
        spookery_engine::testing::journal::blake2s_tagged(&report.now_yaml),
        expected
    );
}

const HUMMED_OIL: &str = "lamp_oil";

#[given(expr = "a pack with a habit rule that requires players == 0")]
fn a_pack_with_a_players_habit(world: &mut World) {
    let pack = crate::support::copy_docks("players-habit");
    let path = pack.join("cast/bram.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read bram: {err}"));
    let habit = "\
rules:
  - id: stock_the_street
    when: habit
    if:
      - players:
          op: \"==\"
          value: 0
    do:
      - item:
          who: place:docks_street
          id: lamp_oil
          add: 1
---
";
    let next = text.replacen(
        "idle:\n  eligible: true\n---\n",
        &format!("idle:\n  eligible: true\n{habit}"),
        1,
    );
    assert_ne!(text, next, "bram's cast did not take the habit");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write bram: {err}"));
    crate::support::stamp_pack_rev(&pack);
    table::listen(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "{string} sits and a round commits")]
fn sits_and_a_round_commits(world: &mut World, actor: String) {
    let pack = table::pack(world).to_path_buf();
    table::seat_actor(world, &pack, &actor);
    a_round_commits(world);
}

#[when(expr = "{string} leaves and a round commits")]
fn leaves_and_a_round_commits(world: &mut World, actor: String) {
    leaves(world, actor);
    a_round_commits(world);
}

#[when(expr = "{string} leaves")]
fn leaves(world: &mut World, actor: String) {
    table::close_actor(world, &actor);
    table::wait_presence(world, &format!("{actor} gone"), |_, actors| {
        !actors.contains(&actor)
    });
    table::live_mut(world)
        .players
        .retain(|player| player.actor_id != actor);
}

/// The GM queues one NPC wait and commits, so a round runs whoever is seated.
#[when(expr = "a round commits")]
fn a_round_commits(world: &mut World) {
    if table::live_mut(world).gm.is_none() {
        table::seat_gm(world);
    }
    let pack = table::pack(world).to_path_buf();
    let before = list_round_keys(&pack).len();
    let queued = table::gm_tool_call(world, 5, "queue", r#"{"actor_id":"aldric","move":"wait"}"#);
    assert!(queued.contains("\"id\""), "{queued}");
    let committed = table::gm_tool_call(world, 6, "commit", r#"{"include_round":true}"#);
    assert_eq!(list_round_keys(&pack).len(), before + 1, "{committed}");
}

fn street_oil(world: &World) -> u32 {
    let now = state_of(table::pack(world));
    now.places
        .get("docks_street")
        .and_then(|place| place.items.get(HUMMED_OIL))
        .copied()
        .unwrap_or(0)
}

#[then(expr = "the habit does not fire")]
fn the_habit_does_not_fire(world: &mut World) {
    assert_eq!(street_oil(world), 0);
}

#[then(expr = "the habit fires")]
fn the_habit_fires(world: &mut World) {
    assert_eq!(street_oil(world), 1);
}

const TABLE_LINE: &str = "the tide turns at the quay";

#[given(expr = "a docks table with {string} seated and {string} not seated")]
fn a_docks_table_with_one_seated_and_one_not(world: &mut World, seated: String, free: String) {
    let pack = crate::support::copy_docks("say-seated");
    table::listen(world, &pack);
    table::seat_actor(world, &pack, &seated);
    world.pack = Some(pack);
    assert!(!table::read_presence(world).1.contains(&free));
}

#[when(expr = "the GM says a line to the table")]
fn the_gm_says_a_line_to_the_table(world: &mut World) {
    if table::live_mut(world).gm.is_none() {
        table::seat_gm(world);
    }
    let args = format!(r#"{{"address":"table","text":"{TABLE_LINE}"}}"#);
    world.scratch = table::gm_tool_call(world, 3, "say", &args);
}

#[then(expr = "{string} hears it and {string} does not")]
fn hears_it_and_does_not(world: &mut World, seated: String, free: String) {
    let said = table::read_reply(&world.scratch);
    assert_eq!(said["recipients"], serde_json::json!([seated]), "{said}");
    let _ = table::wait_actor_from(world, &seated, 0, TABLE_LINE);
    let pack = table::pack(world).to_path_buf();
    table::seat_actor(world, &pack, &free);
    let text = table::actor_text(world, &free);
    assert!(!text.contains(TABLE_LINE), "{free} heard the line\n{text}");
}

#[then(expr = "status shows players {int} and actors_present {string}")]
fn status_shows(world: &mut World, players: u32, actor: String) {
    assert_eq!(table::read_presence(world), (players, vec![actor]));
}

#[then(expr = "the round's presence_players is {int} and actors_present is {string}")]
fn the_rounds_presence_is(world: &mut World, players: u64, actor: String) {
    let pack = table::pack(world);
    let last = list_round_keys(pack)
        .pop()
        .unwrap_or_else(|| panic!("no round was written"));
    let round = crate::support::read_round(pack, &last);
    assert_eq!(
        round["presence_players"],
        serde_json::json!(players),
        "{round}"
    );
    assert_eq!(
        round["actors_present"],
        serde_json::json!([actor]),
        "{round}"
    );
}
