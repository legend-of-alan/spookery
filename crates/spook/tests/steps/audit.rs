use std::fs;

use cucumber::{then, when};
use spookery_kernel::model::Message;

use crate::support::spook;
use crate::World;

use super::table;

const LEDGER: &str = "The lamp-oil ledger is short.";

#[when(expr = "the operator runs audit")]
fn the_operator_runs_audit(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let output = spook(&["audit", &crate::support::pack_arg(&pack)]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[when(expr = "the operator runs audit --strict")]
fn the_operator_runs_audit_strict(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let output = spook(&["audit", "--strict", &crate::support::pack_arg(&pack)]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[when(expr = "two players play, a GM commits a round, ticks pass, and one player drops")]
fn a_normal_docks_session(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    table::seat_actor(&pack, "pc_rowan");
    table::seat_actor(&pack, "pc_harbormouse");
    let _ = table::actor_send("pc_rowan", "wait", "\"kind\":");
    let _ = table::actor_send("pc_harbormouse", "wait", "\"kind\":");
    table::seat_gm();
    let _ = table::actor_send("pc_rowan", "wait", "\"kind\":");
    let _ = table::gm_call(7, "commit_turn", r#"{"include_tick":true}"#);
    table::close_actor("pc_harbormouse");
    table::wait_humans(&pack, 1);
    table::close_actor("pc_rowan");
    table::wait_humans(&pack, 0);
    table::stop_server();
    let arg = pack.to_str().unwrap().to_string();
    for _ in 0..2 {
        table::run(world, &["tick", &arg]);
        assert_eq!(world.code, Some(0), "{}", world.stderr);
    }
}

#[when(expr = "the door note text contains the ledger sentence")]
fn the_door_note_text_contains_the_ledger_sentence(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read moves: {err}"));
    let from = "text: Back at dusk. Oil on the shelf.\n";
    let to = format!("text: Back at dusk. {LEDGER}\n");
    assert!(text.contains(from), "{text}");
    fs::write(&path, text.replacen(from, &to, 1)).unwrap_or_else(|err| panic!("write: {err}"));
    spookery_kernel::commit::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
}

#[when(expr = "the evening tick runs on this copy")]
fn the_evening_tick_runs_on_this_copy(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    spookery_round::play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let now = table::now_of(&pack);
    assert!(
        now.messages
            .iter()
            .any(|message| message.text.contains(LEDGER)),
        "the door note was not delivered\n{:?}",
        now.messages
    );
}

#[when(expr = "a visible note's id is the ledger fact")]
fn a_visible_notes_id_is_the_ledger_fact(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let mut now = table::now_of(&pack);
    now.messages.push(Message {
        id: "fact_ledger_short".to_string(),
        to: vec!["pc_rowan".to_string()],
        place: Some("docks_shop".to_string()),
        fact_ids: Vec::new(),
        text: "The shelf is stocked.".to_string(),
    });
    let text =
        spookery_kernel::store::codec::emit_save(&now).unwrap_or_else(|err| panic!("emit: {err}"));
    fs::write(pack.join("state/save.yaml"), text).unwrap_or_else(|err| panic!("write now: {err}"));
}

#[when(expr = "two players sit and the GM says the ledger fact")]
fn the_gm_says_the_ledger_fact(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    table::seat_actor(&pack, "pc_rowan");
    table::seat_actor(&pack, "pc_harbormouse");
    table::seat_gm();
    let said = table::gm_call(
        4,
        "say",
        &format!(r#"{{"address":"table","text":"{LEDGER}"}}"#),
    );
    assert!(said.contains("\"id\":4"), "{said}");
    table::close_actor("pc_harbormouse");
    table::wait_humans(&pack, 1);
    table::close_actor("pc_rowan");
    table::wait_humans(&pack, 0);
    table::stop_server();
}

#[when(expr = "two players sit and one says the ledger fact")]
fn one_player_says_the_ledger_fact(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    table::seat_actor(&pack, "pc_rowan");
    table::seat_actor(&pack, "pc_harbormouse");
    let _ = table::actor_send("pc_rowan", &format!("say {LEDGER}"), "\"kind\":\"say\"");
    table::close_actor("pc_harbormouse");
    table::wait_humans(&pack, 1);
    table::close_actor("pc_rowan");
    table::wait_humans(&pack, 0);
    table::stop_server();
}

#[then(expr = "the audit exits 0")]
fn the_audit_exits_0(world: &mut World) {
    assert_eq!(world.code, Some(0), "{}\n{}", world.stdout, world.stderr);
}

#[then(expr = "the audit exits 1")]
fn the_audit_exits_1(world: &mut World) {
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
}

#[then(expr = "stdout has a pass line for {string}")]
fn stdout_has_a_pass_line_for(world: &mut World, pc: String) {
    let line = format!("pass {pc}");
    assert!(
        world.stdout.lines().any(|item| item == line),
        "missing {line}\n{}",
        world.stdout
    );
}

#[then(expr = "stdout names {string}")]
fn stdout_names(world: &mut World, text: String) {
    assert!(
        world.stdout.contains(&text),
        "missing {text}\n{}",
        world.stdout
    );
}

#[then(expr = "stdout has a note for speaker {string} and recipient {string}")]
fn stdout_has_a_note_for(world: &mut World, speaker: String, recipient: String) {
    let found = world
        .stdout
        .lines()
        .any(|line| note_matches(line, &speaker, &recipient));
    assert!(
        found,
        "missing note speaker {speaker} recipient {recipient}\n{}",
        world.stdout
    );
}

#[then(expr = "stdout says notes follow the journal")]
fn stdout_says_notes_follow_the_journal(world: &mut World) {
    let line = "notes follow the journal state at the tape line's tick";
    assert!(
        world.stdout.lines().any(|item| item == line),
        "{}\n{}",
        world.stdout,
        world.stderr
    );
}

#[then(expr = "Rowan does not perceive the ledger sentence")]
fn rowan_does_not_perceive_the_ledger_sentence(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let shard = spookery_kernel::World::open(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    let view =
        spookery_kernel::perceive::perceive_at(shard.canon(), shard.now(), "player:pc_rowan")
            .unwrap_or_else(|err| panic!("perceive: {err}"));
    let leaked = view
        .messages
        .iter()
        .any(|message| message.text.contains(LEDGER) || message.id.contains(LEDGER));
    assert!(!leaked, "Rowan saw the ledger sentence\n{view:?}");
    assert!(
        now_has_ledger(&pack),
        "the save lost the door note before the view was checked"
    );
}

fn now_has_ledger(pack: &std::path::Path) -> bool {
    table::now_of(pack)
        .messages
        .iter()
        .any(|message| message.text.contains(LEDGER))
}

fn note_matches(line: &str, speaker: &str, recipient: &str) -> bool {
    let mut parts = line.split_whitespace();
    parts.next() == Some("note")
        && parts.next() == Some("seq")
        && parts.next().is_some()
        && parts.next() == Some("speaker")
        && parts.next() == Some(speaker)
        && parts.next() == Some("recipient")
        && parts.next() == Some(recipient)
        && parts.next().is_none()
}
