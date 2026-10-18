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

#[given(expr = "the docks pack where Mira's door note names {string}")]
fn the_docks_pack_where_miras_door_note_names(world: &mut World, sentence: String) {
    let pack = crate::support::temp_pack("named-note");
    rewrite_mira(
        &pack,
        "          text: Back at dusk. Oil on the shelf.\n",
        &format!("          text: Back at dusk. {sentence}\n"),
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
