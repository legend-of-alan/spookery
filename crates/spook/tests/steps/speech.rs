use std::fs;

use cucumber::{given, then, when};

use crate::support::temp_pack;
use crate::World;

use super::table;

#[given(expr = "a docks table with a seated player")]
fn a_docks_table_with_a_seated_player(world: &mut World) {
    let pack = temp_pack("speech");
    table::listen(&pack);
    table::seat_player(&pack);
    world.pack = Some(pack);
}

#[when(expr = "the player says {string}")]
fn the_player_says(_world: &mut World, text: String) {
    let _ = table::player_send(&format!("say {text}"), "\"kind\":\"say\"");
}

#[when(expr = "say to mira is submitted on an empty table saying {string}")]
fn say_to_mira_on_an_empty_table(world: &mut World, text: String) {
    let pack = table::pack(world);
    let shard =
        spookery_kernel::api::World::open(pack).unwrap_or_else(|err| panic!("World::open: {err}"));
    assert_eq!(shard.now().presence.humans, 0);
    world.now_bytes = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    spookery::host::speak(&shard, "pc_rowan", &format!("say mira {text}"))
        .unwrap_or_else(|err| panic!("speak: {err}"));
}

#[then(expr = "the tape contains the text {string}")]
fn the_tape_contains(world: &mut World, text: String) {
    let tape = tape_text(world);
    assert!(tape.contains(&format!("\"text\":\"{text}\"")), "{tape}");
}

#[then(expr = "the tape address is {string}")]
fn the_tape_address_is(world: &mut World, address: String) {
    let tape = tape_text(world);
    assert!(
        tape.contains(&format!("\"address\":\"{address}\"")),
        "{tape}"
    );
}

#[then(expr = "the tape names the actor {string}")]
fn the_tape_names_the_actor(world: &mut World, actor: String) {
    let tape = tape_text(world);
    assert!(tape.contains(&format!("\"actor\":\"{actor}\"")), "{tape}");
}

#[then(expr = "the tape recipients are {string}")]
fn the_tape_recipients_are(world: &mut World, id: String) {
    let tape = tape_text(world);
    assert!(
        tape.contains(&format!("\"recipients\":[\"{id}\"]")),
        "{tape}"
    );
}

#[then(expr = "the tape names the place {string}")]
fn the_tape_names_the_place(world: &mut World, place: String) {
    let tape = tape_text(world);
    assert!(tape.contains(&format!("\"place\":\"{place}\"")), "{tape}");
}

fn tape_text(world: &World) -> String {
    fs::read_to_string(table::pack(world).join("state/journal/session.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"))
}

#[then(expr = "state_rev is unchanged")]
fn state_rev_is_unchanged(world: &mut World) {
    let rev = crate::with_live(|live| live.seated_rev);
    assert_eq!(table::now_of(table::pack(world)).state_rev, rev);
}

#[then(expr = "save.yaml is unchanged since the seat")]
fn now_unchanged_since_the_seat(world: &mut World) {
    let seated = crate::with_live(|live| live.seated_now.clone());
    let now = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    assert_eq!(now, seated);
}

#[then(expr = "no tick file was written")]
fn no_tick_file(world: &mut World) {
    assert!(table::tick_names(table::pack(world)).is_empty());
}

#[then(expr = "save.yaml matches the bytes from before the line")]
fn now_matches_the_bytes_from_before_the_line(world: &mut World) {
    let now = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    assert_eq!(now, world.now_bytes);
}

#[then(expr = "the tape file is absent")]
fn the_tape_file_is_absent(world: &mut World) {
    assert!(!table::pack(world)
        .join("state/journal/session.jsonl")
        .exists());
}
