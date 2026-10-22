use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_kernel::api::perceive::ItemStack;

use crate::support::spook;
use crate::World;

use super::table;

const CLOCKS: &str = "\
clock assassination_debt 2 6 paused
clock shop_day 1 1 running
";

const DOOR: &str = "effect message 1 {\"message_id\":\"note_mira_door\",\"to\":[\"pc_rowan\"],\"place\":\"docks_shop\",\"fact_ids\":[],\"text\":\"Back at dusk. Oil on the shelf.\"}";

const HOST: &str = "\
effect message 1 {\"message_id\":\"note_mira_door\",\"to\":[\"pc_rowan\"],\"place\":\"docks_shop\",\"fact_ids\":[],\"text\":\"Back at dusk. Oil on the shelf.\"}
clock assassination_debt 2 6 paused
clock shop_day 1 1 running
";

/// The raw player digest after thirteen docks ticks: what `--raw` carries in the resume frame.
pub(crate) const RESUME: &str = "\
message note_mira_door Back at dusk. Oil on the shelf.
clock assassination_debt 2 6 paused
";

#[when(expr = "thirteen ticks pass")]
fn thirteen_ticks_pass(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap();
    for tick in 1..=13 {
        let output = spook(&["tick", arg]);
        assert!(
            output.status.success(),
            "tick {tick}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    world.before = table::files(&pack);
}

#[then(expr = "the host text lists the clocks")]
fn the_host_text_lists_the_clocks(world: &mut World) {
    assert_eq!(world.stdout, CLOCKS);
}

#[then(expr = "the host text lists the door note")]
fn the_host_text_lists_the_door_note(world: &mut World) {
    assert_eq!(world.stdout, HOST);
    let now = table::now_of(table::pack(world));
    assert_eq!(now.tick_index, 13);
    assert_eq!(now.world_time, 1860);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(
        now.places
            .get("docks_shop")
            .and_then(|place| place.items.get("lamp_oil"))
            .copied(),
        Some(4u32)
    );
    let debt = now.clocks.get("assassination_debt").unwrap();
    assert_eq!(debt.value, 2);
    assert!(debt.paused);
}

#[given(expr = "a docks table with {string} seated and no GM")]
fn a_docks_table_with_one_pc_and_no_gm(world: &mut World, actor: String) {
    let pack = crate::support::temp_pack("host-round");
    table::listen(world, &pack);
    table::seat_actor(world, &pack, &actor);
    world.pack = Some(pack);
}

#[given(expr = "{string} has waited once")]
fn has_waited_once(world: &mut World, actor: String) {
    table::actor_send(world, &actor, "wait", "\"kind\":");
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
            .is_some_and(|added| added.contains("clock "))
    });
}

#[then(expr = "the status lists the door note")]
fn the_status_lists_the_door_note(world: &mut World) {
    assert!(world.stdout.contains(DOOR), "{}", world.stdout);
}

#[then(expr = "the host printed the door note exactly once")]
fn the_host_printed_the_door_note_exactly_once(world: &mut World) {
    let text = table::host_stdout(world);
    assert_eq!(text.matches(DOOR).count(), 1, "{text}");
}

#[then(expr = "every host printout ends with the clocks")]
fn every_host_printout_ends_with_the_clocks(world: &mut World) {
    let text = table::host_stdout(world);
    let body = text.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
    // `shop_day` sorts last, so it ends a printout. An effect after that starts the next one.
    let mut in_clocks = false;
    let mut open = false;
    let mut saw = false;
    for line in body.lines().filter(|line| !line.is_empty()) {
        if line.starts_with("effect ") {
            assert!(
                !in_clocks,
                "an effect followed a clock in one printout:\n{line}\n{body}"
            );
            open = true;
        } else if line.starts_with("clock ") {
            in_clocks = true;
            open = true;
            if line.starts_with("clock shop_day ") {
                saw = true;
                in_clocks = false;
                open = false;
            }
        } else {
            panic!("not an effect or a clock:\n{line}\n{body}");
        }
    }
    assert!(
        saw && !open,
        "a printout did not end with the clocks\n{body}"
    );
}

#[then(expr = "the host's last printout lists the clocks and no effect rows")]
fn the_hosts_last_printout_lists_the_clocks(world: &mut World) {
    let text = table::host_stdout(world);
    let added = text.get(world.host_mark..).unwrap_or("");
    let lines: Vec<&str> = added.lines().filter(|line| !line.is_empty()).collect();
    assert!(
        !lines.is_empty() && lines.iter().all(|line| line.starts_with("clock ")),
        "{added}"
    );
}

#[given(expr = "the docks pack where Mira's door note carries the hidden fact {string}")]
fn the_docks_pack_where_miras_door_note_carries(world: &mut World, fact_id: String) {
    let pack = crate::support::temp_pack("hidden-note");
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
    assert_eq!(resume, RESUME);
    assert!(!resume.contains("effect location"));
    assert!(!resume.contains("lamp_oil"));
}

#[then(expr = "the player resume for {string} does not list {string}")]
fn the_player_resume_for_does_not_list(world: &mut World, actor: String, note: String) {
    let pack = table::pack(world);
    let now = table::now_of(pack);
    assert!(
        now.messages.iter().any(|message| message.id == note),
        "the save has no {note}"
    );
    let resume = read_player_resume(world, &actor);
    assert!(!resume.contains(&note), "resume listed {note}:\n{resume}");
}

#[then(expr = "the perceive view for {string} does not list {string}")]
fn the_perceive_view_for_does_not_list(world: &mut World, actor: String, note: String) {
    let pack = table::pack(world);
    let shard = spookery_round::resume(pack).unwrap();
    let view = spookery_kernel::api::perceive(&shard, &format!("player:{actor}")).unwrap();
    assert!(
        view.messages.iter().all(|message| message.id != note),
        "view listed {note}"
    );
}

fn read_player_resume(world: &World, actor: &str) -> String {
    let pack = table::pack(world);
    let shard = spookery_round::resume(pack).unwrap();
    let view = spookery_kernel::api::perceive(&shard, &format!("player:{actor}")).unwrap();
    spookery_kernel::api::player_digest(&view).unwrap()
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
    let shard = spookery_round::resume(pack).unwrap();
    let shop = spookery_kernel::api::perceive(&shard, "player:pc_rowan").unwrap();
    assert_eq!(shop.place, "docks_shop");
    assert_eq!(
        shop.items,
        vec![ItemStack {
            id: "lamp_oil".to_string(),
            count: 4,
        }]
    );
    let mut away = shard.now().clone();
    *away.actors.get_mut("pc_rowan").unwrap().place_mut() = "mira_rooms".to_string();
    let rooms =
        spookery_kernel::api::perceive::perceive_at(shard.canon(), &away, "player:pc_rowan")
            .unwrap();
    assert_eq!(rooms.place, "mira_rooms");
    assert!(rooms.items.iter().all(|item| item.id != "lamp_oil"));
}
