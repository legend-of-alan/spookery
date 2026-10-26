use std::fs;

use cucumber::{given, then};
use spookery_engine::{emit_save, load_save};

use super::table;
use crate::World;

#[given(expr = "rowan starts in docks_street")]
fn rowan_starts_in_docks_street(world: &mut World) {
    let pack = crate::support::temp_pack("arrive");
    let path = pack.join("state/save.yaml");
    let mut now = load_save(&fs::read_to_string(&path).unwrap()).unwrap();
    *now.actors.get_mut("pc_rowan").unwrap().place_mut() = "docks_street".to_string();
    now.places
        .get_mut("docks_shop")
        .unwrap()
        .actors
        .retain(|id| id != "pc_rowan");
    let street = now.places.get_mut("docks_street").unwrap();
    if street.actors.iter().all(|id| id != "pc_rowan") {
        street.actors.push("pc_rowan".to_string());
        street.actors.sort();
    }
    fs::write(&path, emit_save(&now).unwrap()).unwrap();
    world.pack = Some(pack);
}

#[given(expr = "pc_wren is seated and not travelling")]
fn pc_wren_is_seated(world: &mut World) {
    let pack = world.pack.clone().unwrap();
    table::seat_actor(world, &pack, "pc_wren");
}

#[then(expr = "the seated player sees {string}")]
fn the_seated_player_sees(world: &mut World, text: String) {
    let added = if world.player_added.contains(&text) {
        world.player_added.clone()
    } else {
        table::wait_player_from(world, world.mark, &text)
    };
    assert!(added.contains(&text), "{added}");
}
