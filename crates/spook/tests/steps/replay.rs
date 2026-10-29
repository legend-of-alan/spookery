use std::fs;

use cucumber::{given, then, when};
use serde_json::Value;

use crate::World;

use super::steps_run::{read_round, read_steps_run};
use super::table;

#[given(expr = "an empty directory")]
fn an_empty_directory(world: &mut World) {
    world.pack = Some(crate::support::fresh_dir("empty"));
}

#[when(expr = "the operator runs quick verify")]
fn the_operator_runs_quick_verify(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", "--quick", pack.to_str().unwrap()]);
}

#[when(expr = "the pack is copied before the round")]
fn the_pack_is_copied_before_the_round(world: &mut World) {
    let pack = table::pack(world);
    let dest = crate::support::fresh_dir("from");
    crate::support::copy_dir(pack, &dest);
    world.scratch = dest.display().to_string();
}

#[when(expr = "the replay base is removed")]
fn the_replay_base_is_removed(world: &mut World) {
    let path = table::pack(world).join("state/journal/base.yaml");
    if path.is_file() {
        fs::remove_file(&path).unwrap_or_else(|err| panic!("remove base: {err}"));
    }
}

#[when(expr = "the operator runs verify from that copy")]
fn the_operator_runs_verify_from_that_copy(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let from = world.scratch.clone();
    table::run(world, &["verify", pack.to_str().unwrap(), "--from", &from]);
    let _ = fs::remove_dir_all(&from);
}

#[when(expr = "the operator plays {int} rounds")]
fn the_operator_plays_n_rounds(world: &mut World, count: usize) {
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
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_rowan");
    table::seat_actor(world, &pack, "pc_harbormouse");
    let _ = table::actor_send(world, "pc_rowan", "wait", "\"kind\":");
    let _ = table::actor_send(world, "pc_harbormouse", "wait", "\"kind\":");
    table::seat_gm(world);
    let _ = table::actor_send(world, "pc_rowan", "wait", "\"kind\":");
    let _ = table::gm_call(world, 7, "commit_round", r#"{"include_tick":true}"#);
    table::close_actor(world, "pc_harbormouse");
    table::wait_humans(world, &pack, 1);
    table::close_actor(world, "pc_rowan");
    table::wait_humans(world, &pack, 0);
    table::stop_server(world);
}

#[when(expr = "a seated player's server is killed and restarted")]
fn a_seated_player_is_killed_and_restarted(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_rowan");
    table::wait_humans(world, &pack, 1);
    // A hard kill, so the restart has a crashed presence to repair.
    table::live_mut(world).server.take();
    table::stop_server(world);
    table::listen(world, &pack);
    table::wait_humans(world, &pack, 0);
    table::stop_server(world);
}

#[when(expr = "a queued intent the round never had is written into its record")]
fn a_queued_intent_is_written_into_a_round_record(world: &mut World) {
    let mut scratch = String::new();
    edit_round(world, |round| {
        let intents = round["intents"]
            .as_array_mut()
            .unwrap_or_else(|| panic!("no intents"));
        assert!(intents.is_empty(), "a plain round queued {intents:?}");
        intents.push(serde_json::json!({
            "id": "written-in",
            "round_index": 1,
            "actor": "player:pc_rowan",
            "source": "player",
            "move": "take",
            "target": { "actor": "lamp_oil" },
            "args": {},
            "rationale_code": "",
        }));
        let rev = round["state_rev_after"].as_u64().unwrap();
        let index = round["round_index"].as_u64().unwrap();
        scratch = format!("state_rev {rev} round {index}");
    });
    world.scratch = scratch;
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

#[when(expr = "the round record's round_index is a string")]
fn the_round_index_is_a_string(world: &mut World) {
    edit_round(world, |round| {
        round["round_index"] = Value::String("no".to_string());
    });
}

#[when(expr = "the round record gains an unknown key")]
fn the_round_record_gains_an_unknown_key(world: &mut World) {
    edit_round(world, |round| {
        round["surplus"] = Value::Bool(true);
    });
    world.scratch = "surplus".to_string();
}

#[when(expr = "one effect class is need")]
fn one_effect_class_is_need(world: &mut World) {
    let mut scratch = String::new();
    edit_round(world, |round| {
        let effects = round["effects"]
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
        let rev = round["state_rev_after"].as_u64().unwrap();
        let index = round["round_index"].as_u64().unwrap();
        scratch = format!("state_rev {rev} round {index}");
    });
    world.scratch = scratch;
}

#[then(expr = "stderr names the round record and the field round_index")]
fn stderr_names_the_round_field(world: &mut World) {
    assert!(world.stderr.contains("bundle-"), "{}", world.stderr);
    assert!(world.stderr.contains("round_index"), "{}", world.stderr);
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
    assert!(world.stderr.contains("bundle-"), "{}", world.stderr);
}

#[then(expr = "stderr names that record and says ILLEGAL_CLASS")]
fn stderr_names_the_record_and_illegal_class(world: &mut World) {
    assert!(world.stderr.contains("ILLEGAL_CLASS"), "{}", world.stderr);
    assert!(
        world.stderr.contains("bundle-") || world.stderr.contains(world.scratch.trim()),
        "{}",
        world.stderr
    );
}

#[then(expr = "each journal record parses back to the bytes that were written")]
fn each_journal_record_parses_back(world: &mut World) {
    let pack = table::pack(world);
    let entries = spookery_engine::testing::journal::read_verified(pack).unwrap_or_else(|fault| {
        panic!("{}: {}", fault.file, fault.detail);
    });
    let rounds = entries
        .iter()
        .filter(|entry| entry.round_index().is_some())
        .count();
    assert!(rounds >= 3, "rounds {rounds}");
    for entry in &entries {
        let key = spookery_engine::testing::journal::format_record_key(entry);
        let located = spookery_engine::testing::journal::locate(pack, &key)
            .unwrap_or_else(|err| panic!("{}", err.detail));
        let again = spookery_engine::testing::journal::emit_entry(entry);
        let text = fs::read_to_string(located.path).unwrap();
        assert!(
            text.lines().any(|line| line == again.trim_end()),
            "{key}: {again}"
        );
    }
}

fn edit_round(world: &mut World, edit: impl FnOnce(&mut Value)) {
    let located = spookery_engine::testing::journal::locate(table::pack(world), "round-0001")
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let text = fs::read_to_string(&located.path).unwrap();
    let line = text
        .lines()
        .find(|l| serde_json::from_str::<Value>(l).unwrap()["round_index"] == 1)
        .unwrap();
    let mut round: Value = serde_json::from_str(line).unwrap();
    edit(&mut round);
    let next = serde_json::to_string(&round).unwrap();
    let rewritten = text.replacen(line, &next, 1);
    assert_ne!(rewritten, text);
    fs::write(&located.path, rewritten).unwrap();
}

#[given(expr = "checkpoints are taken every {int} rounds")]
fn checkpoints_are_taken_every_n_rounds(world: &mut World, every: u64) {
    let path = table::pack(world).join("pack.yaml");
    let mut text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read pack.yaml: {err}"));
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("checkpoint_every: {every}\n"));
    fs::write(&path, text).unwrap();
}

#[when(expr = "the live bundle gains a torn tail")]
fn the_live_bundle_gains_a_torn_tail(world: &mut World) {
    let pack = table::pack(world);
    let path = spookery_engine::testing::journal::find_live_bundle(pack)
        .unwrap_or_else(|| panic!("no live bundle"));
    let entries = spookery_engine::testing::journal::read_entries(pack)
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let last = entries
        .iter()
        .rev()
        .find(|entry| entry.changes_state())
        .unwrap_or_else(|| panic!("no round"));
    world.mark = last.state_rev_after() as usize;
    world.scratch = last.now_hash().to_string();
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend(br#"{"torn":"#);
    fs::write(&path, bytes).unwrap();
}

#[when(expr = "the table is opened again")]
fn the_table_is_opened_again(world: &mut World) {
    resume_as_writer(table::pack(world));
}

/// Take the lock the way a host does, which recovers a crashed pack, then resume it.
fn resume_as_writer(pack: &std::path::Path) {
    let _lock = spookery_engine::lock_pack(pack).unwrap_or_else(|err| panic!("lock: {err}"));
    spookery_engine::World::resume(pack).unwrap_or_else(|err| panic!("resume: {err}"));
}

#[then(expr = "the torn tail is gone")]
fn the_torn_tail_is_gone(world: &mut World) {
    let path = spookery_engine::testing::journal::find_live_bundle(table::pack(world))
        .unwrap_or_else(|| panic!("no live bundle"));
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.ends_with('\n'), "{text}");
    assert!(!text.contains("{\"torn\":"), "{text}");
}

#[then(expr = "the opened state matches the last complete record")]
fn the_opened_state_matches_the_last_complete_record(world: &mut World) {
    let report = spookery_engine::testing::replay_journal(table::pack(world), None, None)
        .unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(report.state_rev, world.mark as u64);
    assert_eq!(
        spookery_engine::testing::journal::hash_now(&report.now_yaml),
        world.scratch
    );
}

#[when(expr = "one record in a closed bundle is changed")]
fn one_record_in_a_closed_bundle_is_changed(world: &mut World) {
    let pack = table::pack(world);
    let bundles = spookery_engine::testing::journal::bundles(pack)
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let (name, path) = bundles
        .into_iter()
        .find(|(name, _)| name.ends_with(".gz"))
        .unwrap_or_else(|| panic!("no closed bundle"));
    let compressed = fs::read(&path).unwrap();
    let plain = spookery_engine::testing::journal::gunzip_bytes(&compressed)
        .unwrap_or_else(|err| panic!("gunzip: {err}"));
    let text = String::from_utf8(plain).unwrap_or_else(|err| panic!("{err}"));
    let next = spookery_engine::testing::journal::gzip_bytes(&change_payload(&text))
        .unwrap_or_else(|err| panic!("gzip: {err}"));
    fs::write(&path, next).unwrap();
    world.scratch = name;
}

#[when(expr = "one record in the live bundle is changed")]
fn one_record_in_the_live_bundle_is_changed(world: &mut World) {
    let pack = table::pack(world);
    let path = spookery_engine::testing::journal::find_live_bundle(pack)
        .unwrap_or_else(|| panic!("no live bundle"));
    let name = path
        .file_name()
        .and_then(|file| file.to_str())
        .unwrap_or("bundle")
        .to_string();
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read: {err}"));
    fs::write(&path, change_payload(&text)).unwrap();
    world.scratch = name;
}

/// Change a value the chain hash covers, not the stored hash itself.
fn change_payload(text: &str) -> Vec<u8> {
    let needle = "\"world_time_before\":";
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("no world_time_before\n{text}"))
        + needle.len();
    let mut bytes = text.as_bytes().to_vec();
    bytes[at] = if bytes[at] == b'9' { b'8' } else { b'9' };
    bytes
}

#[when(expr = "every closed bundle is made unreadable")]
fn every_closed_bundle_is_made_unreadable(world: &mut World) {
    let bundles = spookery_engine::testing::journal::bundles(table::pack(world))
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let closed: Vec<_> = bundles
        .into_iter()
        .filter(|(name, _)| name.ends_with(".gz"))
        .collect();
    assert!(!closed.is_empty(), "no closed bundle");
    for (_name, path) in closed {
        fs::write(&path, b"not a gzip bundle").unwrap();
    }
}

#[then(expr = "the table still opens")]
fn the_table_still_opens(world: &mut World) {
    spookery_engine::World::resume(table::pack(world)).unwrap_or_else(|err| panic!("open: {err}"));
}

#[then(expr = "stderr names that bundle")]
fn stderr_names_that_bundle(world: &mut World) {
    assert!(
        world.stderr.contains(world.scratch.trim()),
        "missing {}\n{}",
        world.scratch,
        world.stderr
    );
}

#[when(expr = "the save snapshot is removed")]
fn the_save_snapshot_is_removed(world: &mut World) {
    let path = table::pack(world).join("state/save.yaml");
    assert!(path.is_file(), "no save");
    fs::remove_file(&path).unwrap();
}

#[when(expr = "the operator rebuilds the snapshot")]
fn the_operator_rebuilds_the_snapshot(world: &mut World) {
    resume_as_writer(table::pack(world));
}

#[then(expr = "the save hash matches the latest checkpoint")]
fn the_save_hash_matches_the_latest_checkpoint(world: &mut World) {
    let pack = table::pack(world);
    let entries = spookery_engine::testing::journal::read_entries(pack)
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let mark = spookery_engine::testing::journal::latest_checkpoint(&entries)
        .unwrap_or_else(|| panic!("no checkpoint"));
    let save = fs::read(pack.join("state/save.yaml")).unwrap();
    assert_eq!(
        spookery_engine::testing::journal::hash_now(&save),
        mark.state_hash
    );
}

#[then(expr = "the first round lists only effect_applied and cast:mira\\/leave_door_note")]
fn the_first_round_keeps_only_the_extra_steps(world: &mut World) {
    let round = read_round(table::pack(world), "round-0001");
    assert_eq!(
        read_steps_run(&round),
        ["effect_applied", "cast:mira/leave_door_note"]
    );
}

#[then(expr = "the second round omits steps_run")]
fn the_second_round_omits_steps_run(world: &mut World) {
    let round = read_round(table::pack(world), "round-0002");
    assert!(round.get("steps_run").is_none(), "{round}");
}

#[when(expr = "the operator plays a round")]
fn the_operator_plays_a_round(world: &mut World) {
    the_operator_plays_n_rounds(world, 1);
}

#[when(expr = "the closed bundle is expanded back to plain JSONL")]
fn the_closed_bundle_is_expanded_back_to_plain_jsonl(world: &mut World) {
    let (name, path) = only_closed_bundle(world);
    let plain = gunzip_closed(&path);
    fs::write(path.with_file_name(name.trim_end_matches(".gz")), plain).unwrap();
    fs::remove_file(&path).unwrap();
}

#[when(expr = "the closed bundle is copied back beside its gzip")]
fn the_closed_bundle_is_copied_back_beside_its_gzip(world: &mut World) {
    let (name, path) = only_closed_bundle(world);
    let plain = gunzip_closed(&path);
    fs::write(path.with_file_name(name.trim_end_matches(".gz")), plain).unwrap();
    assert!(path.is_file(), "gzip removed");
}

#[when(expr = "the save is kept")]
fn the_save_is_kept(world: &mut World) {
    let save = table::pack(world).join("state/save.yaml");
    world.scratch = fs::read_to_string(save).unwrap_or_else(|err| panic!("read save: {err}"));
}

/// Leave the journal as a commit whose `save.yaml` and snapshot did not land.
#[when(expr = "the checkpoint's save and snapshot are rolled back")]
fn the_checkpoints_save_and_snapshot_are_rolled_back(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let entries = spookery_engine::testing::journal::read_entries(&pack)
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let mark = spookery_engine::testing::journal::latest_checkpoint(&entries)
        .unwrap_or_else(|| panic!("no checkpoint"));
    let snap = pack
        .join("state/checkpoints")
        .join(format!("{}.yaml", mark.state_rev));
    fs::remove_file(snap).unwrap_or_else(|err| panic!("remove snapshot: {err}"));
    fs::write(pack.join("state/save.yaml"), &world.scratch).unwrap();
    the_closed_bundle_is_expanded_back_to_plain_jsonl(world);
}

#[then(expr = "the live bundle is {word}")]
fn the_live_bundle_is(world: &mut World, expected: String) {
    let path = spookery_engine::testing::journal::find_live_bundle(table::pack(world))
        .unwrap_or_else(|| panic!("no live bundle"));
    let name = path
        .file_name()
        .and_then(|file| file.to_str())
        .unwrap_or("");
    assert_eq!(name, expected);
}

#[then(expr = "{word} is closed")]
fn the_bundle_is_closed(world: &mut World, name: String) {
    let journal = table::pack(world).join("state/journal");
    assert!(journal.join(format!("{name}.gz")).is_file(), "no gzip");
    assert!(!journal.join(&name).exists(), "plain bundle still open");
}

#[then(expr = "the plain bundle file is gone")]
fn the_plain_bundle_file_is_gone(world: &mut World) {
    let journal = table::pack(world).join("state/journal");
    let mut plain = Vec::new();
    for entry in fs::read_dir(&journal).unwrap_or_else(|err| panic!("read journal: {err}")) {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy().into_owned();
        if name.ends_with(".jsonl") {
            plain.push(name);
        }
    }
    assert!(plain.is_empty(), "{plain:?}");
}

fn only_closed_bundle(world: &World) -> (String, std::path::PathBuf) {
    let bundles = spookery_engine::testing::journal::bundles(table::pack(world))
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let mut closed: Vec<_> = bundles
        .into_iter()
        .filter(|(name, _)| name.ends_with(".gz"))
        .collect();
    assert_eq!(closed.len(), 1, "{closed:?}");
    closed.pop().unwrap()
}

fn gunzip_closed(path: &std::path::Path) -> Vec<u8> {
    let compressed = fs::read(path).unwrap_or_else(|err| panic!("read gzip: {err}"));
    spookery_engine::testing::journal::gunzip_bytes(&compressed)
        .unwrap_or_else(|err| panic!("gunzip: {err}"))
}

#[when(expr = "the first closed checkpoint snapshot is changed")]
fn the_first_closed_checkpoint_snapshot_is_changed(world: &mut World) {
    let dir = table::pack(world).join("state/checkpoints");
    let first = fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("read checkpoints: {err}"))
        .filter_map(|entry| {
            entry
                .ok()?
                .path()
                .file_stem()?
                .to_str()?
                .parse::<u64>()
                .ok()
        })
        .min()
        .unwrap_or_else(|| panic!("no checkpoint snapshot"));
    let path = dir.join(format!("{first}.yaml"));
    let mut text = fs::read(&path).unwrap();
    text.extend_from_slice(b"# changed\n");
    fs::write(&path, text).unwrap();
    // verify names the bundle whose checkpoint it checked.
    world.scratch = "bundle-0000.jsonl.gz: state hash differed".into();
}
