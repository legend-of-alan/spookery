use spookery_engine::testing::world_of;
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery::protocol::ItemStack;
use spookery_engine::{component, GlobalId, Meter, Note, Presence};

use crate::support::run_spook;
use crate::World;

use super::host;

const METERS: &str = "\
meter rowan assassination_debt 2 6
meter place:docks_shop comfort 2 3
meter mira shop_day 1 1
";

/// `shop_day` sorts last, so its meter line ends every host printout.
const PRINTOUT_END: &str = "meter mira shop_day ";

const DOOR_NOTE: &str = "effect note 1 {\"note_id\":\"note_mira_door\",\"to_ids\":[\"rowan\"],\"place_id\":\"docks_shop\",\"text\":\"Back at dusk. Oil on the shelf.\"}";

const HOST: &str = "\
effect note 1 {\"note_id\":\"note_mira_door\",\"to_ids\":[\"rowan\"],\"place_id\":\"docks_shop\",\"text\":\"Back at dusk. Oil on the shelf.\"}
meter rowan assassination_debt 2 6
meter place:docks_shop comfort 2 3
meter mira shop_day 1 1
";

#[when(expr = "thirteen rounds pass")]
async fn thirteen_rounds_pass(world: &mut World) {
    let pack = host::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap();
    for round in 1..=13 {
        let output = run_spook(&["round", arg]).await;
        assert!(
            output.status.success(),
            "round {round}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    world.before = host::read_files(&pack);
}

#[then(expr = "the host text lists the meters")]
fn the_host_text_lists_the_meters(world: &mut World) {
    assert_eq!(world.stdout, METERS);
}

#[then(expr = "the host text lists the door note")]
fn the_host_text_lists_the_door_note(world: &mut World) {
    assert_eq!(world.stdout, HOST);
    let now = world_of(host::pack(world));
    assert_eq!(now.round_index, 13);
    assert_eq!(now.world_time, 1860);
    assert_eq!(now.last_round_at, "2026-01-01T00:00:00Z");
    assert_eq!(
        now.places()
            .get::<component::Item>(&"docks_shop".into())
            .get("lamp_oil")
            .copied(),
        Some(4u32)
    );
    let rowan = "rowan".into();
    let meters = now.actors().get::<Meter>(&rowan);
    let debt = meters.get_or_base("assassination_debt").unwrap();
    assert_eq!((debt.base, debt.offset), (2, 0));
}

#[given(expr = "a docks table with {string} seated and no GM")]
async fn a_docks_table_with_one_pc_and_no_gm(world: &mut World, actor: String) {
    let pack = crate::support::copy_pack("docks", "host-round");
    host::listen(world, &pack).await;
    let printed = host::stdout(world).matches(PRINTOUT_END).count();
    host::seat_actor(world, &pack, &actor).await;
    host::wait_until(world, "the seat's printout", |text| {
        text.matches(PRINTOUT_END).count() > printed
    })
    .await;
    world.pack = Some(pack);
}

#[given(expr = "{string} has waited once")]
async fn has_waited_once(world: &mut World, actor: String) {
    let printed = host::stdout(world).matches(PRINTOUT_END).count();
    host::actor_send(world, &actor, "wait", "\"kind\":").await;
    host::wait_until(world, "the wait's printout", |text| {
        text.matches(PRINTOUT_END).count() > printed
    })
    .await;
}

#[when(expr = "{string} waits three times")]
async fn waits_three_times(world: &mut World, actor: String) {
    for _ in 0..3 {
        host::actor_send(world, &actor, "wait", "\"kind\":").await;
    }
}

#[when(expr = "{string} closes the seat")]
async fn closes_the_seat(world: &mut World, actor: String) {
    let mark = host::stdout(world).len();
    world.host_mark = mark;
    host::close_actor(world, &actor).await;
    host::wait_until(world, "seat close", |text| {
        text.get(mark..)
            .is_some_and(|added| added.contains("meter "))
    })
    .await;
}

#[then(expr = "the status lists the door note")]
fn the_status_lists_the_door_note(world: &mut World) {
    assert!(world.stdout.contains(DOOR_NOTE), "{}", world.stdout);
}

#[then(expr = "the host printed the door note exactly once")]
fn the_host_printed_the_door_note_exactly_once(world: &mut World) {
    let text = host::stdout(world);
    assert_eq!(text.matches(DOOR_NOTE).count(), 1, "{text}");
}

#[then(expr = "every host printout ends with the meters")]
fn every_host_printout_ends_with_the_meters(world: &mut World) {
    let text = host::stdout(world);
    let body = text.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
    let mut in_meters = false;
    let mut open = false;
    let mut saw = false;
    for line in body.lines().filter(|line| !line.is_empty()) {
        if line.starts_with("effect ") {
            assert!(
                !in_meters,
                "an effect followed a meter in one printout:\n{line}\n{body}"
            );
            open = true;
        } else if line.starts_with("meter ") {
            in_meters = true;
            open = true;
            if line.starts_with(PRINTOUT_END) {
                saw = true;
                in_meters = false;
                open = false;
            }
        } else {
            panic!("not an effect or a meter:\n{line}\n{body}");
        }
    }
    assert!(
        saw && !open,
        "a printout did not end with the meters\n{body}"
    );
}

#[then(expr = "the host's last printout lists the meters and no effect rows")]
fn the_hosts_last_printout_lists_the_meters(world: &mut World) {
    let text = host::stdout(world);
    let added = text.get(world.host_mark..).unwrap_or("");
    let lines: Vec<&str> = added.lines().filter(|line| !line.is_empty()).collect();
    assert!(
        !lines.is_empty() && lines.iter().all(|line| line.starts_with("meter ")),
        "{added}"
    );
}

#[given(expr = "the docks pack where Mira's door note is left to Wren in Mira's rooms")]
fn the_docks_pack_where_miras_door_note_is_left_to_wren(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "hidden-note");
    rewrite_mira(
        &pack,
        "            - rowan\n          place_id: docks_shop\n",
        "            - wren\n          place_id: mira_rooms\n",
    );
    world.pack = Some(pack);
}

#[then(expr = "the player resume lists the door message")]
async fn the_player_resume_lists_the_door_message(world: &mut World) {
    let resume = read_player_resume(world, "rowan").await;
    assert_eq!(
        resume,
        serde_json::json!({
            "kind": "resume",
            "notes": [
                {
                    "id": "note_mira_door",
                    "to_ids": ["rowan"],
                    "place_id": "docks_shop",
                    "text": "Back at dusk. Oil on the shelf."
                }
            ]
        })
        .to_string()
    );
    assert!(!resume.contains("effect location"));
    assert!(!resume.contains("lamp_oil"));
}

#[then(expr = "the player resume for {string} does not list {string}")]
async fn the_player_resume_for_does_not_list(world: &mut World, actor: String, note: String) {
    let pack = host::pack(world);
    let now = world_of(pack);
    let notes = now.globals().get::<Note>(&GlobalId);
    assert!(notes.contains(&note), "the save has no {note}");
    let resume = read_player_resume(world, &actor).await;
    assert!(!resume.contains(&note), "resume listed {note}:\n{resume}");
}

#[then(expr = "the perceive view for {string} does not list {string}")]
fn the_perceive_view_for_does_not_list(world: &mut World, actor: String, note: String) {
    let pack = host::pack(world);
    let opened = spookery_engine::Session::peek(pack).unwrap();
    let view = opened.perceive(&Presence::default(), &actor).unwrap();
    assert!(
        view.notes.iter().all(|message| message.id != note),
        "view listed {note}"
    );
}

/// The resume frame the host sends when `actor` is claimed on a host running this pack.
async fn read_player_resume(world: &mut World, actor: &str) -> String {
    let pack = host::pack(world).to_path_buf();
    host::listen(world, &pack).await;
    host::seat_actor(world, &pack, actor).await;
    let text = host::actor_text(world, actor);
    host::stop(world);
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .map(|message| message["body"].clone())
        .find(|body| body["kind"] == "resume")
        .unwrap_or_else(|| panic!("no resume frame\n{text}"))
        .to_string()
}

fn rewrite_mira(pack: &Path, from: &str, to: &str) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read mira: {err}"));
    let next = text.replacen(from, to, 1);
    assert_ne!(next, text, "Mira's door note was not rewritten");
    fs::write(&path, next).unwrap_or_else(|err| panic!("write mira: {err}"));
    crate::support::stamp_pack_rev(pack);
}

#[then(expr = "the shop lists the oil and the rooms do not")]
fn the_shop_lists_the_oil(world: &mut World) {
    let pack = host::pack(world);
    let opened = spookery_engine::Session::peek(pack).unwrap();
    let shop = opened.perceive(&Presence::default(), "rowan").unwrap();
    assert_eq!(shop.place, "docks_shop");
    assert_eq!(
        shop.items,
        vec![ItemStack {
            id: "lamp_oil".to_string(),
            count: 4,
            name: "Lamp oil".to_string(),
        }]
    );
    let rooms = opened.perceive(&Presence::default(), "mira").unwrap();
    assert_eq!(rooms.place, "mira_rooms");
    assert!(rooms.items.iter().all(|item| item.id != "lamp_oil"));
}
