use cucumber::{then, when};
use spookery_kernel::perceive::StockView;
use spookery_kernel::DigestViewer;

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

const RESUME: &str = "\
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
            .and_then(|place| place.stock.get("lamp_oil"))
            .copied(),
        Some(4)
    );
    let debt = now.clocks.get("assassination_debt").unwrap();
    assert_eq!(debt.value, 2);
    assert!(debt.paused);
}

#[then(expr = "the player resume lists the door message")]
fn the_player_resume_lists_the_door_message(world: &mut World) {
    let pack = table::pack(world);
    let shard = spookery_kernel::open_shard(pack).unwrap();
    let resume =
        spookery_kernel::idle_digest(&shard, DigestViewer::Player("pc_rowan".to_string())).unwrap();
    assert_eq!(resume, RESUME);
    assert!(!resume.contains("effect location"));
    assert!(!resume.contains("lamp_oil"));
}

#[then(expr = "the shop lists the oil and the rooms do not")]
fn the_shop_lists_the_oil(world: &mut World) {
    let pack = table::pack(world);
    let shard = spookery_kernel::open_shard(pack).unwrap();
    let shop = spookery_kernel::perceive(&shard, "player:pc_rowan").unwrap();
    assert_eq!(shop.place, "docks_shop");
    assert_eq!(
        shop.stocks,
        vec![StockView {
            sku: "lamp_oil".to_string(),
            qty: 4,
        }]
    );
    let mut away = shard.now().clone();
    *away.actors.get_mut("pc_rowan").unwrap().place_mut() = "mira_rooms".to_string();
    let rooms =
        spookery_kernel::perceive::perceive_at(shard.canon(), &away, "player:pc_rowan").unwrap();
    assert_eq!(rooms.place, "mira_rooms");
    assert!(rooms.stocks.iter().all(|stock| stock.sku != "lamp_oil"));
}
