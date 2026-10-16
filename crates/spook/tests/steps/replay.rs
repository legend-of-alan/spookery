use std::fs;

use cucumber::{given, then, when};
use serde_json::Value;

use crate::World;

use super::table;

#[given(expr = "an empty directory")]
fn an_empty_directory(world: &mut World) {
    let dir = std::env::temp_dir().join(format!(
        "spook-empty-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("mkdir {}: {err}", dir.display()));
    world.pack = Some(dir);
}

#[when(expr = "the operator runs verify")]
fn the_operator_runs_verify(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", pack.to_str().unwrap()]);
}

#[when(expr = "the pack is copied before the tick")]
fn the_pack_is_copied_before_the_tick(world: &mut World) {
    let pack = table::pack(world);
    let dest = std::env::temp_dir().join(format!(
        "spook-from-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dest);
    crate::support::copy_dir(pack, &dest);
    world.scratch = dest.display().to_string();
}

#[when(expr = "the replay base is removed")]
fn the_replay_base_is_removed(world: &mut World) {
    let path = table::pack(world).join("state/journal/base.yaml");
    assert!(path.is_file(), "no replay base at {}", path.display());
    fs::remove_file(&path).unwrap_or_else(|err| panic!("remove base: {err}"));
}

#[when(expr = "the operator runs verify from that copy")]
fn the_operator_runs_verify_from_that_copy(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let from = world.scratch.clone();
    table::run(world, &["verify", pack.to_str().unwrap(), "--from", &from]);
    let _ = fs::remove_dir_all(&from);
}

#[when(expr = "the operator runs {int} ticks")]
fn the_operator_runs_n_ticks(world: &mut World, count: usize) {
    let pack = table::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap().to_string();
    for _ in 0..count {
        table::run(world, &["tick", &arg]);
        assert_eq!(world.code, Some(0), "{}", world.stderr);
    }
}

#[when(expr = "two players play, a GM commits one round, and the players leave")]
fn two_players_play_a_round(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    table::seat_actor(&pack, "pc_rowan");
    table::seat_actor(&pack, "pc_harbormouse");
    let _ = table::actor_send("pc_rowan", "wait", "\"kind\":");
    let _ = table::actor_send("pc_harbormouse", "wait", "\"kind\":");
    table::seat_gm();
    let _ = table::actor_send("pc_rowan", "wait", "\"kind\":");
    let _ = table::gm_call(7, "commit_round", r#"{"include_tick":true}"#);
    table::close_actor("pc_harbormouse");
    table::wait_humans(&pack, 1);
    table::close_actor("pc_rowan");
    table::wait_humans(&pack, 0);
    table::stop_server();
}

#[when(expr = "a seated player's server is killed and restarted")]
fn a_seated_player_is_killed_and_restarted(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    table::seat_actor(&pack, "pc_rowan");
    table::wait_humans(&pack, 1);
    table::stop_server();
    table::listen(&pack);
    table::wait_humans(&pack, 0);
    table::stop_server();
}

#[when(expr = "one intent in a tick record is changed")]
fn one_intent_in_a_tick_record_is_changed(world: &mut World) {
    let path = table::pack(world).join("state/journal/tick-0001.jsonl");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read tick: {err}"));
    let mut tick: Value =
        serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("json: {err}"));
    let intents = tick["intents"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("no intents\n{text}"));
    assert!(!intents.is_empty(), "{text}");
    intents[0]["move"] = Value::String("not_a_real_move".to_string());
    let rev = tick["state_rev_after"]
        .as_u64()
        .unwrap_or_else(|| panic!("{text}"));
    let index = tick["tick_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("{text}"));
    world.scratch = format!("state_rev {rev} tick {index}");
    fs::write(&path, serde_json::to_string(&tick).unwrap()).unwrap();
}

#[when(expr = "the pack bytes are remembered")]
fn the_pack_bytes_are_remembered(world: &mut World) {
    world.before = table::files(table::pack(world));
}

#[then(expr = "stdout says the save was verified")]
fn stdout_says_the_save_was_verified(world: &mut World) {
    let rev = table::now_of(table::pack(world)).state_rev;
    assert!(
        world.stdout.contains("verified ") && world.stdout.contains("records, state_rev "),
        "{}",
        world.stdout
    );
    assert!(
        world.stdout.contains(&format!("state_rev {rev}")),
        "stdout {} live {rev}",
        world.stdout
    );
}

#[then(expr = "stdout shows the final state_rev")]
fn stdout_shows_the_final_state_rev(world: &mut World) {
    let rev = table::now_of(table::pack(world)).state_rev;
    assert!(
        world.stdout.contains(&format!("state_rev {rev}")),
        "{}",
        world.stdout
    );
    assert!(world.stdout.contains("records"), "{}", world.stdout);
}

#[then(expr = "stderr names that record")]
fn stderr_names_that_record(world: &mut World) {
    assert!(
        world.stderr.contains(world.scratch.trim()),
        "missing {}\n{}",
        world.scratch,
        world.stderr
    );
}

#[then(expr = "stderr names the first differing save line")]
fn stderr_names_the_first_differing_save_line(world: &mut World) {
    assert!(world.stderr.contains("save.yaml:"), "{}", world.stderr);
}

#[when(expr = "the tick record's tick_index is a string")]
fn the_tick_index_is_a_string(world: &mut World) {
    let path = tick_path(world);
    let mut tick = tick_value(&path);
    tick["tick_index"] = Value::String("no".to_string());
    write_tick(&path, &tick);
}

#[when(expr = "the tick record gains an unknown key")]
fn the_tick_record_gains_an_unknown_key(world: &mut World) {
    let path = tick_path(world);
    let mut tick = tick_value(&path);
    tick["surplus"] = Value::Bool(true);
    write_tick(&path, &tick);
    world.scratch = "surplus".to_string();
}

#[when(expr = "one effect class is need")]
fn one_effect_class_is_need(world: &mut World) {
    let path = tick_path(world);
    let mut tick = tick_value(&path);
    let effects = tick["effects"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("no effects"));
    if effects.is_empty() {
        effects.push(serde_json::json!({
            "class": "need",
            "as": "npc:mira",
            "intent_id": "i",
        }));
    } else {
        effects[0]["class"] = Value::String("need".to_string());
    }
    let rev = tick["state_rev_after"].as_u64().unwrap_or(1);
    let index = tick["tick_index"].as_u64().unwrap_or(1);
    world.scratch = format!("state_rev {rev} tick {index}");
    write_tick(&path, &tick);
}

#[then(expr = "stderr names the tick record and the field tick_index")]
fn stderr_names_the_tick_field(world: &mut World) {
    assert!(world.stderr.contains("tick-0001.jsonl"), "{}", world.stderr);
    assert!(world.stderr.contains("tick_index"), "{}", world.stderr);
    assert!(world.stderr.contains("invalid type"), "{}", world.stderr);
    assert!(
        world.stderr.contains("line") && world.stderr.contains("column"),
        "{}",
        world.stderr
    );
    assert!(!world.stderr.contains("is missing"), "{}", world.stderr);
}

#[then(expr = "stderr names that unknown key")]
fn stderr_names_that_unknown_key(world: &mut World) {
    assert!(
        world.stderr.contains(&world.scratch),
        "missing {}\n{}",
        world.scratch,
        world.stderr
    );
    assert!(world.stderr.contains("tick-0001.jsonl"), "{}", world.stderr);
}

#[then(expr = "stderr names that record and says ILLEGAL_CLASS")]
fn stderr_names_the_record_and_illegal_class(world: &mut World) {
    assert!(world.stderr.contains("ILLEGAL_CLASS"), "{}", world.stderr);
    assert!(
        world.stderr.contains("tick-0001.jsonl") || world.stderr.contains(world.scratch.trim()),
        "{}",
        world.stderr
    );
}

#[then(expr = "each journal record parses back to the bytes that were written")]
fn each_journal_record_parses_back(world: &mut World) {
    let pack = table::pack(world);
    let entries = spookery_kernel::api::journal::load_journal(pack).unwrap_or_else(|fault| {
        panic!("{}: {}", fault.file, fault.detail);
    });
    let ticks = entries
        .iter()
        .filter(|entry| entry.tick_index().is_some())
        .count();
    assert!(ticks >= 3, "ticks {ticks}");
    for entry in &entries {
        let bytes = fs::read_to_string(pack.join(journal_file(entry))).unwrap();
        let again = spookery_kernel::api::journal::emit_entry(entry);
        assert_eq!(again, bytes, "{bytes}");
    }
}

fn tick_path(world: &World) -> std::path::PathBuf {
    table::pack(world).join("state/journal/tick-0001.jsonl")
}

fn tick_value(path: &std::path::Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|err| panic!("read tick: {err}"));
    serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("json: {err}"))
}

fn write_tick(path: &std::path::Path, tick: &Value) {
    let mut text = serde_json::to_string(tick).unwrap();
    text.push('\n');
    fs::write(path, text).unwrap();
}

fn journal_file(entry: &spookery_kernel::api::journal::JournalEntry) -> String {
    match entry.tick_index() {
        Some(index) => format!("state/journal/tick-{index:04}.jsonl"),
        None => format!(
            "state/journal/{}-{:04}.json",
            entry.kind_name(),
            entry.state_rev_after()
        ),
    }
}
