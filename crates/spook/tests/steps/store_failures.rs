use std::fs;

use cucumber::when;
use serde_json::Value;

use crate::World;

use super::table;

/// A directory where the tape file goes, so opening it fails on every platform.
#[when(expr = "the tape cannot be written")]
fn the_tape_cannot_be_written(world: &mut World) {
    let tape = table::pack(world).join("state/tape.jsonl");
    let _ = fs::remove_file(&tape);
    fs::create_dir(&tape).unwrap_or_else(|err| panic!("tape dir: {err}"));
}

/// A directory where the live bundle was, so reading it fails on every platform.
#[when(expr = "the live journal bundle cannot be read")]
fn the_live_journal_bundle_cannot_be_read(world: &mut World) {
    let path = spookery_engine::testing::journal::find_live_bundle(table::pack(world))
        .unwrap_or_else(|| panic!("no live bundle"));
    fs::remove_file(&path).unwrap_or_else(|err| panic!("remove bundle: {err}"));
    fs::create_dir(&path).unwrap_or_else(|err| panic!("bundle dir: {err}"));
}

#[when(expr = "the player tries to say {string}")]
fn the_player_tries_to_say(world: &mut World, text: String) {
    let (mark, added) = table::player_send(world, &format!("say {text}"), "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

/// Rewrites the last live record without resealing it, so its chain hash no longer covers it.
#[when(expr = "the last journal record is changed under the table")]
fn the_last_journal_record_is_changed(world: &mut World) {
    let path = spookery_engine::testing::journal::find_live_bundle(table::pack(world))
        .unwrap_or_else(|| panic!("no live bundle"));
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read bundle: {err}"));
    let last = text
        .lines()
        .last()
        .unwrap_or_else(|| panic!("empty bundle"));
    let mut record: Value = serde_json::from_str(last).unwrap_or_else(|err| panic!("{err}"));
    record["state_hash"] = Value::from("changed");
    let changed = text.replacen(last, &record.to_string(), 1);
    assert_ne!(changed, text);
    fs::write(&path, changed).unwrap_or_else(|err| panic!("write bundle: {err}"));
}
