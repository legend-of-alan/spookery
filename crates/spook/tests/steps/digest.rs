use spookery_engine::testing::state_of;
use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery::protocol::ItemStack;
use spookery_engine::Presence;

use crate::support::run_spook;
use crate::World;

use super::table;

const METERS: &str = "\
meter pc_rowan assassination_debt 2 6
meter mira shop_day 1 1
";

/// `shop_day` sorts last, so its meter line ends every host printout.
const PRINTOUT_END: &str = "meter mira shop_day ";

const DOOR_NOTE: &str = "effect note 1 {\"note_id\":\"note_mira_door\",\"to\":[\"pc_rowan\"],\"place\":\"docks_shop\",\"fact_ids\":[],\"text\":\"Back at dusk. Oil on the shelf.\"}";

const HOST: &str = "\
effect note 1 {\"note_id\":\"note_mira_door\",\"to\":[\"pc_rowan\"],\"place\":\"docks_shop\",\"fact_ids\":[],\"text\":\"Back at dusk. Oil on the shelf.\"}
meter pc_rowan assassination_debt 2 6
meter mira shop_day 1 1
";

#[when(expr = "thirteen rounds pass")]
fn thirteen_rounds_pass(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap();
    for round in 1..=13 {
        let output = run_spook(&["round", arg]);
        assert!(
            output.status.success(),
            "round {round}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    world.before = table::read_files(&pack);
}

#[then(expr = "the host text lists the meters")]
fn the_host_text_lists_the_meters(world: &mut World) {
    assert_eq!(world.stdout, METERS);
}

#[then(expr = "the host text lists the door note")]
fn the_host_text_lists_the_door_note(world: &mut World) {
    assert_eq!(world.stdout, HOST);
    let now = state_of(table::pack(world));
    assert_eq!(now.round_index, 13);
    assert_eq!(now.world_time, 1860);
    assert_eq!(now.last_round_at, "2026-01-01T00:00:00Z");
    assert_eq!(
        now.item_count(
            &spookery_engine::EntityId::Place("docks_shop".into()),
            "lamp_oil"
        )
        .filter(|count| *count > 0),
        Some(4u32)
    );
    let rowan = spookery_engine::EntityId::Actor("pc_rowan".into());
    let debt = now.meters_of(&rowan).get("assassination_debt").unwrap();
    assert_eq!((debt.base, debt.offset), (2, 0));
}

#[given(expr = "a docks table with {string} seated and no GM")]
fn a_docks_table_with_one_pc_and_no_gm(world: &mut World, actor: String) {
    let pack = crate::support::copy_docks("host-round");
    table::listen(world, &pack);
    let printed = table::host_stdout(world).matches(PRINTOUT_END).count();
    table::seat_actor(world, &pack, &actor);
    // The host prints the seat change after the player's view.
    table::wait_host_until(world, "the seat's printout", |text| {
        text.matches(PRINTOUT_END).count() > printed
    });
    world.pack = Some(pack);
}

#[given(expr = "{string} has waited once")]
fn has_waited_once(world: &mut World, actor: String) {
    let printed = table::host_stdout(world).matches(PRINTOUT_END).count();
    table::actor_send(world, &actor, "wait", "\"kind\":");
    // The host prints the round after the player's reply.
    table::wait_host_until(world, "the wait's printout", |text| {
        text.matches(PRINTOUT_END).count() > printed
    });
}

#[when(expr = "{string} waits three times")]
fn waits_three_times(world: &mut World, actor: String) {
    for _ in 0..3 {
        table::actor_send(world, &actor, "wait", "\"kind\":");
    }
}

#[when(expr = "{string} closes the seat")]
fn closes_the_seat(world: &mut World, actor: String) {
    let mark = table::host_stdout(world).len();
    world.host_mark = mark;
    table::close_actor(world, &actor);
    table::wait_host_until(world, "seat close", |text| {
        text.get(mark..)
            .is_some_and(|added| added.contains("meter "))
    });
}

#[then(expr = "the status lists the door note")]
fn the_status_lists_the_door_note(world: &mut World) {
    assert!(world.stdout.contains(DOOR_NOTE), "{}", world.stdout);
}

#[then(expr = "the host printed the door note exactly once")]
fn the_host_printed_the_door_note_exactly_once(world: &mut World) {
    let text = table::host_stdout(world);
    assert_eq!(text.matches(DOOR_NOTE).count(), 1, "{text}");
}

#[then(expr = "every host printout ends with the meters")]
fn every_host_printout_ends_with_the_meters(world: &mut World) {
    let text = table::host_stdout(world);
    let body = text.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
    // An effect after a printout's end starts the next one.
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
    let text = table::host_stdout(world);
    let added = text.get(world.host_mark..).unwrap_or("");
    let lines: Vec<&str> = added.lines().filter(|line| !line.is_empty()).collect();
    assert!(
        !lines.is_empty() && lines.iter().all(|line| line.starts_with("meter ")),
        "{added}"
    );
}

#[given(expr = "the docks pack where Mira's door note carries the hidden fact {string}")]
fn the_docks_pack_where_miras_door_note_carries(world: &mut World, fact_id: String) {
    let pack = crate::support::copy_docks("hidden-note");
    rewrite_mira(
        &pack,
        "          fact_ids: []\n",
        &format!("          fact_ids:\n            - {fact_id}\n"),
    );
    world.pack = Some(pack);
}

#[then(expr = "the player resume lists the door message")]
fn the_player_resume_lists_the_door_message(world: &mut World) {
    let resume = read_player_resume(world, "pc_rowan");
    assert_eq!(
        resume,
        serde_json::json!({
            "kind": "resume",
            "notes": [
                {
                    "id": "note_mira_door",
                    "to": ["pc_rowan"],
                    "place": "docks_shop",
                    "fact_ids": [],
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
fn the_player_resume_for_does_not_list(world: &mut World, actor: String, note: String) {
    let pack = table::pack(world);
    let now = state_of(pack);
    assert!(
        now.notes().iter().any(|message| message.id == note),
        "the save has no {note}"
    );
    let resume = read_player_resume(world, &actor);
    assert!(!resume.contains(&note), "resume listed {note}:\n{resume}");
}

#[then(expr = "the perceive view for {string} does not list {string}")]
fn the_perceive_view_for_does_not_list(world: &mut World, actor: String, note: String) {
    let pack = table::pack(world);
    let opened = spookery_engine::World::peek(pack).unwrap();
    let view = opened.perceive(&Presence::default(), &actor).unwrap();
    assert!(
        view.notes.iter().all(|message| message.id != note),
        "view listed {note}"
    );
}

#[given(expr = "the docks pack where {string} is written without hidden")]
fn the_docks_pack_where_a_fact_has_no_hidden(world: &mut World, fact_id: String) {
    let pack = crate::support::copy_docks("no-hidden");
    let shop = fs::read_to_string(pack.join("world/places/docks_shop.md")).unwrap();
    let (_, row) = shop
        .split_once(&format!("  - id: {fact_id}\n"))
        .unwrap_or_else(|| panic!("the shop has no {fact_id}"));
    let row = row.split("  - id:").next().unwrap_or(row);
    assert!(!row.contains("hidden:"), "{fact_id} has a hidden key");
    world.pack = Some(pack);
}

#[given(expr = "{string} is in {string}")]
fn is_in(world: &mut World, actor: String, place: String) {
    let now = state_of(table::pack(world));
    assert_eq!(now.actors().records()[&actor].place, place);
}

#[then(expr = "the perceive view for {string} lists the fact {string}")]
fn the_perceive_view_lists_the_fact(world: &mut World, actor: String, fact_id: String) {
    assert!(
        view_has_fact(world, &actor, &fact_id),
        "view lacks {fact_id}"
    );
}

#[then(expr = "the perceive view for {string} does not list the fact {string}")]
fn the_perceive_view_does_not_list_the_fact(world: &mut World, actor: String, fact_id: String) {
    assert!(
        !view_has_fact(world, &actor, &fact_id),
        "view listed {fact_id}"
    );
}

fn view_has_fact(world: &World, actor: &str, fact_id: &str) -> bool {
    let opened = spookery_engine::World::peek(table::pack(world)).unwrap();
    let view = opened.perceive(&Presence::default(), actor).unwrap();
    view.known_facts.iter().any(|fact| fact.id == fact_id)
}

/// The resume frame the host sends when `actor` sits at a table on this pack.
fn read_player_resume(world: &mut World, actor: &str) -> String {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    table::seat_actor(world, &pack, actor);
    let text = table::actor_text(world, actor);
    table::stop_host(world);
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
    let pack = table::pack(world);
    let opened = spookery_engine::World::peek(pack).unwrap();
    let shop = opened.perceive(&Presence::default(), "pc_rowan").unwrap();
    assert_eq!(shop.place, "docks_shop");
    assert_eq!(
        shop.items,
        vec![ItemStack {
            id: "lamp_oil".to_string(),
            count: 4,
        }]
    );
    let rooms = opened.perceive(&Presence::default(), "mira").unwrap();
    assert_eq!(rooms.place, "mira_rooms");
    assert!(rooms.items.iter().all(|item| item.id != "lamp_oil"));
}
