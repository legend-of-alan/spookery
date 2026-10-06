use std::fs;

use cucumber::{given, then, when};

use crate::support::temp_pack;
use crate::World;

use super::table;

#[given(expr = "a docks table with a seated player")]
fn a_docks_table_with_a_seated_player(world: &mut World) {
    let pack = temp_pack("speech");
    let port = table::free_port();
    let bind = format!("127.0.0.1:{port}");
    table::listen(&pack, &bind, port);
    table::seat_player(&pack);
    world.pack = Some(pack);
}

#[when(expr = "the player says {string}")]
fn the_player_says(_world: &mut World, text: String) {
    table::say(&format!("say {text}"), &text);
}

#[when(expr = "talk to mira is submitted on an empty table saying {string}")]
fn talk_on_an_empty_table(world: &mut World, text: String) {
    let pack = table::pack(world);
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    assert_eq!(shard.now().presence.humans, 0);
    let _ = spookery::seat::speak(&shard, "pc_rowan", &format!("talk mira {text}"));
}

#[then(expr = "the tape contains the text {string}")]
fn the_tape_contains(world: &mut World, text: String) {
    let tape = fs::read_to_string(table::pack(world).join("state/log/session.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"));
    assert!(tape.contains(&format!("\"text\":\"{text}\"")), "{tape}");
}

#[then(expr = "state_rev is unchanged")]
fn state_rev_is_unchanged(world: &mut World) {
    let rev = crate::with_live(|live| live.seated_rev);
    assert_eq!(table::now_of(table::pack(world)).state_rev, rev);
}

#[then(expr = "now.yaml is unchanged since the seat")]
fn now_unchanged_since_the_seat(world: &mut World) {
    let seated = crate::with_live(|live| live.seated_now.clone());
    let now = fs::read(table::pack(world).join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    assert_eq!(now, seated);
}

#[then(expr = "no tick file was written")]
fn no_tick_file(world: &mut World) {
    assert!(table::tick_names(table::pack(world)).is_empty());
}

#[then(expr = "the tape file is absent")]
fn the_tape_file_is_absent(world: &mut World) {
    assert!(!table::pack(world).join("state/log/session.jsonl").exists());
}
