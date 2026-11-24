use spookery_engine::testing::{act_of, world_of};
use spookery_engine::{component, GlobalId, Note};
use std::fs;

use cucumber::{given, then, when};

use crate::World;

use super::table;

#[when(expr = "a second player connects")]
async fn a_second_player_connects(world: &mut World) {
    world.scratch = table::claim(world, "rowan").await;
}

#[when(expr = "a second GM connects on the game port")]
async fn a_second_gm_connects_on_the_game_port(world: &mut World) {
    let seat = table::Seat::open(table::live_mut(world).game_port, "gm").await;
    world.scratch = table::wait_suffix(&seat.text, 0, "\"kind\":\"refused\"").await;
    seat.wait_closed().await;
}

#[when(expr = "an unclaimed player waits")]
async fn an_unclaimed_player_waits(world: &mut World) {
    if world.live.is_none() {
        listen_pack(world).await;
    }
    let seat = table::Seat::open(table::live_mut(world).game_port, "player").await;
    let id = seat.send("wait", serde_json::json!({}));
    world.scratch = seat.wait_reply(&id).await;
}

#[when(expr = "a player claims an empty id")]
async fn a_player_claims_an_empty_id(world: &mut World) {
    world.scratch = table::claim(world, "").await;
}

#[when(expr = "a player claims {word}")]
async fn a_player_claims(world: &mut World, actor: String) {
    listen_pack(world).await;
    world.scratch = table::claim(world, &actor).await;
}

#[then(expr = "the hello reply does not contain {string}")]
fn the_hello_reply_does_not_contain(world: &mut World, needle: String) {
    assert!(!world.scratch.contains(&needle), "{}", world.scratch);
}

#[when(expr = "the operator runs play on that table")]
async fn the_operator_runs_play_on_that_table(world: &mut World) {
    let addr = table::game_addr(world);
    table::run(world, &["play", "--addr", &addr]).await;
}

#[then(expr = "save.yaml is unchanged since the table opened")]
fn save_unchanged_since_open(world: &mut World) {
    let opened = table::live_mut(world).opened_save.clone();
    let now = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    assert_eq!(now, opened);
}

#[then(expr = "players is {int}")]
async fn players_is(world: &mut World, count: u32) {
    assert_eq!(table::read_presence(world).await.0, count);
}

#[then(expr = "the present actor is {word}")]
async fn the_present_actor_is(world: &mut World, actor: String) {
    assert_eq!(table::read_presence(world).await.1, vec![actor]);
}

#[then(expr = "state_rev is 0")]
fn state_rev_is_0(world: &mut World) {
    assert_eq!(world_of(table::pack(world)).state_rev, 0);
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

#[when(expr = "a player claims a PC")]
async fn a_player_claims_a_pc(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack).await;
    table::seat_actor(world, &pack, "rowan").await;
}

#[when(expr = "the player leaves")]
async fn the_player_leaves(world: &mut World) {
    table::close_actor(world, "rowan").await;
    table::wait_players(world, 0).await;
}

#[when(expr = "the host restarts")]
async fn the_host_restarts(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::stop_host(world);
    table::listen(world, &pack).await;
}

#[then(expr = "nobody is present")]
async fn nobody_is_present(world: &mut World) {
    let (players, actors) = table::read_presence(world).await;
    assert_eq!(players, 0);
    assert!(actors.is_empty(), "{actors:?}");
}

const DOCKS_PCS: [&str; 3] = ["harbormouse", "rowan", "wren"];
const PRIVATE_NOTE_ID: &str = "note_wren_channel";
const PRIVATE_NOTE_TEXT: &str = "The tide chart marks a private channel.";

#[when(expr = "three players claim and a GM connects")]
async fn three_players_claim_and_a_gm_connects(world: &mut World) {
    listen_pack(world).await;
    table::seat_gm(world).await;
    let pack = table::pack(world).to_path_buf();
    seat_three(world, &pack).await;
}

#[given(expr = "a docks table whose pack defines four PCs")]
fn a_docks_table_whose_pack_defines_four_pcs(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "four-pcs");
    let cast = "\
---
schema: cast
id: lark
control_type: pc_template
name: Lark
idle:
  eligible: false
---
Lark watches the street from a dry doorway.
";
    fs::write(pack.join("cast/pc-lark.md"), cast).unwrap_or_else(|err| panic!("cast: {err}"));
    crate::support::edit_save(&pack, |now| {
        let (from, to) = (&"wren".into(), &"lark".into());
        let mut actors = now.actors_mut();
        actors.copy_actor(from, to, "docks_street").unwrap();
    });
    spookery_engine::Session::open(&pack).unwrap_or_else(|err| panic!("four-pc pack: {err}"));
    world.pack = Some(pack);
}

#[when(expr = "three players claim")]
async fn three_players_claim(world: &mut World) {
    listen_pack(world).await;
    let pack = table::pack(world).to_path_buf();
    seat_three(world, &pack).await;
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
async fn the_present_actors_are_the_three_docks_pcs(world: &mut World) {
    assert_eq!(
        table::read_presence(world).await.1,
        DOCKS_PCS.map(str::to_string).to_vec()
    );
}

#[given(expr = "a one-player docks table")]
async fn a_one_player_docks_table(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "one-pc");
    fs::remove_file(pack.join("cast/pc-harbormouse.md"))
        .unwrap_or_else(|err| panic!("remove harbormouse: {err}"));
    fs::remove_file(pack.join("cast/pc-wren.md"))
        .unwrap_or_else(|err| panic!("remove wren: {err}"));
    crate::support::edit_save(&pack, |now| {
        let mut actors = now.actors_mut();
        actors.remove_actor(&"harbormouse".into()).unwrap();
        actors.remove_actor(&"wren".into()).unwrap();
    });
    spookery_engine::Session::open(&pack).unwrap_or_else(|err| panic!("one-pc pack: {err}"));
    table::listen(world, &pack).await;
    table::seat_actor(world, &pack, "rowan").await;
    world.pack = Some(pack);
}

#[given(expr = "a docks table with three seated players")]
async fn a_docks_table_with_three_seated_players(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "three-players");
    table::listen(world, &pack).await;
    seat_three(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "the first player drops")]
async fn the_first_player_drops(world: &mut World) {
    table::close_actor(world, "rowan").await;
    table::wait_players(world, 2).await;
}

#[then(expr = "the present actors are harbormouse and wren")]
async fn the_present_actors_are_harbormouse_and_wren(world: &mut World) {
    assert_eq!(
        table::read_presence(world).await.1,
        vec!["harbormouse".to_string(), "wren".to_string()]
    );
}

#[given(expr = "a docks table where one PC has a private note")]
fn a_docks_table_where_one_pc_has_a_private_note(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "private-note");
    crate::support::edit_save(&pack, |now| {
        assert!(now.actors().contains(&"wren".into()), "wren");
        let note = Note {
            to_ids: ["wren".into()].into(),
            place_id: None,
            text: PRIVATE_NOTE_TEXT.to_string(),
        };
        let mut globals = now.globals_mut();
        (globals.get_mut::<Note>(&GlobalId))
            .leave(PRIVATE_NOTE_ID, note)
            .unwrap();
    });
    world.pack = Some(pack);
}

#[then(expr = "only that PC's perceive shows the private note")]
fn only_that_pcs_perceive_shows_the_private_note(world: &mut World) {
    let wren = table::actor_text(world, "wren");
    assert!(wren.contains(PRIVATE_NOTE_ID), "{wren}");
    for id in ["rowan", "harbormouse"] {
        let text = table::actor_text(world, id);
        assert!(!text.contains(PRIVATE_NOTE_ID), "{id}\n{text}");
    }
}

#[when(expr = "that PC says the note's words and another PC waits")]
async fn that_pc_says_the_notes_words(world: &mut World) {
    let marks = ["rowan", "harbormouse"].map(|id| (id, table::actor_text(world, id).len()));
    let say = format!("say {PRIVATE_NOTE_TEXT}");
    table::actor_send(world, "wren", &say, "\"kind\":\"say\"").await;
    table::actor_send(world, "rowan", "wait", "\"kind\":\"view\"").await;
    for (id, mark) in marks {
        let heard = table::wait_actor_from(world, id, mark, "\"kind\":\"view\"").await;
        assert!(
            heard.contains(PRIVATE_NOTE_TEXT),
            "{id} did not hear it\n{heard}"
        );
    }
}

#[given(expr = "a docks table with two seated players")]
async fn a_docks_table_with_two_seated_players(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "two-live");
    table::listen(world, &pack).await;
    table::seat_actor(world, &pack, "rowan").await;
    table::seat_actor(world, &pack, "harbormouse").await;
    world.pack = Some(pack);
}

#[when(expr = "both players wait")]
async fn both_players_wait(world: &mut World) {
    let ids: Vec<String> = table::live_mut(world)
        .players
        .iter()
        .map(|player| player.actor_id.clone())
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    for id in ids {
        world.player_added = table::actor_send(world, &id, "wait", "\"kind\":").await;
    }
}

#[then(expr = "each wait committed its own round")]
async fn each_wait_committed_its_own_round(world: &mut World) {
    table::wait_round(world, 1090).await;
    let now = world_of(table::pack(world));
    assert_eq!(now.round_index, 2);
    assert_eq!(act_of(&now, "rowan").last_act_round, 1);
    assert_eq!(act_of(&now, "harbormouse").last_act_round, 2);
}

#[given(expr = "a docks table with two PCs in the shop")]
async fn a_docks_table_with_two_pcs_in_the_shop(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "two-shop");
    crate::support::edit_save(&pack, |now| {
        let mut actors = now.actors_mut();
        actors
            .get_mut::<component::Place>(&"harbormouse".into())
            .set("docks_shop")
            .unwrap();
    });
    table::listen(world, &pack).await;
    seat_three(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "the shop player speaks to the place")]
async fn the_shop_player_speaks_to_the_place(world: &mut World) {
    let rowan = &table::player_client(world, "rowan").seat;
    let say = serde_json::json!({
        "to": { "place": "docks_shop" },
        "text": "the blue rope is tied",
    });
    let id = rowan.send("say", say);
    rowan.wait_reply(&id).await;
    table::poll_until("harbormouse to hear the speech in the place", async || {
        let heard = table::actor_text(world, "harbormouse");
        if heard.contains("the blue rope is tied") {
            Ok(())
        } else {
            Err(heard)
        }
    })
    .await;
}

#[then(expr = "only the shop players receive the speech")]
async fn only_the_shop_players_receive_the_speech(world: &mut World) {
    assert!(table::actor_text(world, "rowan").contains("the blue rope is tied"));
    assert!(table::actor_text(world, "harbormouse").contains("the blue rope is tied"));
    let wren = &table::player_client(world, "wren").seat;
    let id = wren.send("look", serde_json::json!({}));
    wren.wait_reply(&id).await;
    let wren = table::actor_text(world, "wren");
    assert!(!wren.contains("the blue rope is tied"), "{wren}");
}

async fn listen_pack(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack).await;
}

async fn seat_three(world: &mut World, pack: &std::path::Path) {
    for id in ["rowan", "harbormouse", "wren"] {
        table::seat_actor(world, pack, id).await;
    }
}

const HUMMED_OIL: &str = "lamp_oil";

#[given(expr = "a pack with a habit rule that requires players == 0")]
async fn a_pack_with_a_players_habit(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "players-habit");
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
          of: place:docks_street
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
    table::listen(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "{string} claims and a round commits")]
async fn claims_and_a_round_commits(world: &mut World, actor: String) {
    let pack = table::pack(world).to_path_buf();
    table::seat_actor(world, &pack, &actor).await;
    a_round_commits(world).await;
}

#[when(expr = "{string} leaves and a round commits")]
async fn leaves_and_a_round_commits(world: &mut World, actor: String) {
    leaves(world, actor).await;
    a_round_commits(world).await;
}

#[when(expr = "{string} leaves")]
async fn leaves(world: &mut World, actor: String) {
    table::close_actor(world, &actor).await;
    table::wait_presence(world, &format!("{actor} gone"), |_, actors| {
        !actors.contains(&actor)
    })
    .await;
    table::live_mut(world)
        .players
        .retain(|player| player.actor_id != actor);
}

/// The GM queues one NPC wait and commits, so a round runs whoever is seated.
#[when(expr = "a round commits")]
async fn a_round_commits(world: &mut World) {
    if table::live_mut(world).gm.is_none() {
        table::seat_gm(world).await;
    }
    let pack = table::pack(world).to_path_buf();
    let before = world_of(&pack).round_index;
    let queued =
        table::gm_tool_call(world, 5, "queue", r#"{"actor":"aldric","move":"wait"}"#).await;
    assert!(queued.contains("\"id\""), "{queued}");
    let committed = table::gm_tool_call(world, 6, "commit", r#"{"include_round":true}"#).await;
    assert_eq!(world_of(&pack).round_index, before + 1, "{committed}");
}

fn street_oil(world: &World) -> u32 {
    let now = world_of(table::pack(world));
    now.places()
        .get::<component::Item>(&"docks_street".into())
        .get(HUMMED_OIL)
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
async fn a_docks_table_with_one_seated_and_one_not(
    world: &mut World,
    seated: String,
    free: String,
) {
    let pack = crate::support::copy_pack("docks", "say-seated");
    table::listen(world, &pack).await;
    table::seat_actor(world, &pack, &seated).await;
    world.pack = Some(pack);
    assert!(!table::read_presence(world).await.1.contains(&free));
}

#[when(expr = "the GM says a line to the table")]
async fn the_gm_says_a_line_to_the_table(world: &mut World) {
    if table::live_mut(world).gm.is_none() {
        table::seat_gm(world).await;
    }
    let args = format!(r#"{{"to":"table","text":"{TABLE_LINE}"}}"#);
    world.scratch = table::gm_tool_call(world, 3, "say", &args).await;
}

#[then(expr = "{string} hears it and {string} does not")]
async fn hears_it_and_does_not(world: &mut World, seated: String, free: String) {
    let speech = table::read_reply(&world.scratch);
    assert_eq!(
        speech["recipient_ids"],
        serde_json::json!([seated]),
        "{speech}"
    );
    let _ = table::wait_actor_from(world, &seated, 0, TABLE_LINE).await;
    let pack = table::pack(world).to_path_buf();
    table::seat_actor(world, &pack, &free).await;
    let text = table::actor_text(world, &free);
    assert!(!text.contains(TABLE_LINE), "{free} heard the line\n{text}");
}

#[then(expr = "status shows players {int} and actors_present {string}")]
async fn status_shows(world: &mut World, players: u32, actor: String) {
    assert_eq!(table::read_presence(world).await, (players, vec![actor]));
}
