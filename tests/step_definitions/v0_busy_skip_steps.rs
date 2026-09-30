use std::fs;

use cucumber::{given, then};

use crate::World;

#[given(expr = "the player {word} is present with {word} in docks_shop and world_time {int}")]
fn player_present(w: &mut World, player: String, npc: String, world_time: u64) {
    let pack = w.pack.as_ref().unwrap();
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let mut now = shard.now().clone();
    drop(shard);
    now.presence.humans = 1;
    now.presence.actors_present = vec![player];
    now.world_time = world_time;
    *now.actors
        .get_mut(&npc)
        .unwrap_or_else(|| panic!("{npc} missing"))
        .place_mut() = "docks_shop".to_string();
    let text =
        spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit_now: {err}"));
    fs::write(pack.join("state/now.yaml"), text).unwrap_or_else(|err| panic!("write now: {err}"));
}

#[then(expr = "world_time advanced to {int}")]
fn world_time_advanced(w: &mut World, n: u64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    assert_eq!(
        shard.now().world_time, n,
        "a present player still advances world_time"
    );
}

#[then(expr = "mira place is docks_shop")]
fn mira_place(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    assert_eq!(shard.now().actors.get("mira").unwrap().place(), "docks_shop");
}

#[then(expr = "mira last_act_tick is {int}")]
fn mira_last_act(w: &mut World, n: u64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    assert_eq!(
        shard
            .now()
            .actors
            .get("mira")
            .unwrap()
            .tick()
            .unwrap()
            .last_act_tick,
        n,
        "mira files no idle move while the player is in docks_shop"
    );
}

#[then(expr = "docks_shop lamp_oil is {int}")]
fn lamp_oil_is(w: &mut World, qty: i64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    assert_eq!(
        shard
            .now()
            .places
            .get("docks_shop")
            .and_then(|place| place.stock.get("lamp_oil"))
            .copied(),
        Some(qty)
    );
}

#[then(expr = "bram place is docks_shop")]
fn bram_place(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    assert_eq!(
        shard.now().actors.get("bram").unwrap().place(),
        "docks_shop",
        "an npc in another place still acts"
    );
}

#[then(expr = "bram last_act_tick is {int}")]
fn bram_last_act(w: &mut World, n: u64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    assert_eq!(
        shard
            .now()
            .actors
            .get("bram")
            .unwrap()
            .tick()
            .unwrap()
            .last_act_tick,
        n
    );
}
