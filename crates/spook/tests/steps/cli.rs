use std::io::Write;

use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use spookery_engine::testing::stamp_pack_rev;
use spookery_engine::Meter;

use crate::support;
use crate::World;

use super::common::{
    a_docks_table, add_verb, capture_play_stdout, edit_save, play_as_pc, replace_in_file,
    spawn_play, wait_play, wait_play_until,
};
use super::host;

/// The blank-line separated blocks of a step's docstring.
pub(crate) fn split_blocks(step: &Step) -> Vec<&str> {
    let text = step.docstring.as_deref().unwrap_or_default();
    text.trim().split("\n\n").collect()
}

#[when(expr = "the operator asks spook for help")]
async fn the_operator_asks_spook_for_help(world: &mut World) {
    host::run(world, &["--help"]).await;
}

#[when(expr = "the operator asks the {string} command for help")]
async fn the_operator_asks_a_command_for_help(world: &mut World, command: String) {
    host::run(world, &[&command, "--help"]).await;
}

#[when(expr = "the operator runs the arguments {string} and {string}")]
async fn the_operator_runs_two_arguments(world: &mut World, first: String, second: String) {
    host::run(world, &[&first, &second]).await;
}

#[then(expr = "stdout shows usage")]
fn stdout_shows_usage(world: &mut World) {
    assert!(world.stdout.contains("Usage:"), "{}", world.stdout);
}

#[then(expr = "stderr shows usage")]
fn stderr_shows_usage(world: &mut World) {
    assert!(world.stderr.contains("Usage:"), "{}", world.stderr);
    assert_ne!(world.stderr.trim(), "NOT_IN_SPEC", "{}", world.stderr);
}

#[then(expr = "stdout names the default loopback address")]
fn stdout_names_the_default_loopback_address(world: &mut World) {
    assert!(world.stdout.contains("127.0.0.1:9737"), "{}", world.stdout);
}

#[when(expr = "play joins the table with --plain")]
async fn play_joins_the_table_with_plain(world: &mut World) {
    play_as_pc(world, "rowan", &["--plain"]).await;
}

#[when(expr = "play joins the table with raw frames")]
async fn play_joins_the_table_with_raw_frames(world: &mut World) {
    play_as_pc(world, "rowan", &["--raw"]).await;
}

#[when(expr = "play says {string}")]
fn play_says(world: &mut World, text: String) {
    let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
    let stdin = play
        .stdin
        .as_mut()
        .unwrap_or_else(|| panic!("play stdin is closed"));
    writeln!(stdin, "say {text}").unwrap_or_else(|err| panic!("say: {err}"));
    stdin.flush().unwrap_or_else(|err| panic!("flush: {err}"));
}

#[when(expr = "another play picks {word} on that table")]
async fn another_play_picks_on_that_table(world: &mut World, pc: String) {
    let port = host::live_mut(world).game_port;
    let addr = format!("127.0.0.1:{port}");
    let mut play = spawn_play(&addr, &pc, &[]);
    let (status, stdout, stderr) = play.wait_with_output().await;
    world.code = status.code();
    world.stdout = stdout;
    world.stderr = stderr;
}

#[then(expr = "play shows each of these blocks:")]
async fn play_shows_each_block(world: &mut World, step: &Step) {
    for block in split_blocks(step) {
        wait_play(world, block).await;
    }
}

#[then(expr = "play does not show {string}")]
fn play_does_not_show(world: &mut World, text: String) {
    let stdout = capture_play_stdout(world);
    assert!(!stdout.contains(&text), "{stdout}");
}

#[then(expr = "play does not show the line {string}")]
fn play_does_not_show_the_line(world: &mut World, text: String) {
    let stdout = capture_play_stdout(world);
    assert!(stdout.lines().all(|line| line != text), "{stdout}");
}

#[then(expr = "play's error shows {string}")]
async fn play_error_shows(world: &mut World, text: String) {
    let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
    world.stderr = host::poll_until(&text, async || {
        let stderr = support::text_of(&play.stderr);
        if stderr.contains(&text) {
            Ok(stderr)
        } else {
            Err(stderr)
        }
    })
    .await;
}

#[then(expr = "play shows protocol frames")]
async fn play_shows_protocol_frames(world: &mut World) {
    wait_play(world, "\"kind\":\"view\"").await;
    let stdout = capture_play_stdout(world);
    assert!(stdout.contains("\"kind\":\"resume\""), "{stdout}");
    assert!(stdout.contains('{'), "{stdout}");
}

#[then(expr = "play shows the door note as a sentence")]
async fn play_shows_the_readable_resume(world: &mut World) {
    let away = "\
While you were away:
Back at dusk. Oil on the shelf.
";
    wait_play(world, away).await;
    let stdout = capture_play_stdout(world);
    assert!(!stdout.contains("note_mira_door"), "{stdout}");
    assert!(!stdout.contains("note "), "{stdout}");
}

#[then(expr = "the round shows no away heading")]
async fn the_round_shows_no_away_heading(world: &mut World) {
    wait_play_until(
        world,
        |stdout| stdout.matches("You are in").count() >= 2,
        "a second You are in",
    )
    .await;
    let stdout = capture_play_stdout(world);
    let after = stdout.split_once("You are in").unwrap().1;
    assert!(!after.contains("While you were away:"), "{stdout}");
}

#[then(expr = "the round's update shows the door note only under Notes")]
async fn the_rounds_update_shows_the_door_note_only_under_notes(world: &mut World) {
    let note = "Back at dusk. Oil on the shelf.";
    wait_play_until(
        world,
        |stdout| stdout.matches("You are in").count() >= 2,
        "a second You are in",
    )
    .await;
    let stdout = capture_play_stdout(world);
    let second = stdout
        .match_indices("You are in")
        .nth(1)
        .map(|(index, _)| index)
        .unwrap_or_else(|| panic!("no second place\n{stdout}"));
    let head = &stdout[..second];
    let gap = head
        .rfind("Exits:")
        .map(|index| &head[index..])
        .unwrap_or(head);
    assert!(
        !gap.contains(note) && !gap.contains("Assassination debt"),
        "a digest row sits above the place\n{stdout}"
    );
    let update = &stdout[second..];
    assert!(update.contains(&format!("Notes:\n{note}")), "{stdout}");
    assert_eq!(update.matches(note).count(), 1, "{stdout}");
}

#[then(expr = "play shows earlier speech under one heading")]
fn play_shows_earlier_speech_under_one_heading(world: &mut World) {
    let shown = capture_play_stdout(world);
    let (picker, stdout) = shown.split_at(shown.find("Earlier:").unwrap_or(0));
    assert!(picker.starts_with("Choose a character:\n"), "{shown}");
    let earlier = "\
Earlier:
You: hello Briar

";
    assert!(stdout.starts_with(earlier), "{stdout}");
    assert_eq!(stdout.matches("Earlier:").count(), 1, "{stdout}");
    assert_eq!(stdout.matches("You: hello Briar").count(), 1, "{stdout}");
    let live = &stdout[earlier.len()..];
    assert!(live.starts_with("You are in Docks Shop."), "{stdout}");
}

#[then(expr = "play shows the raw resume digest")]
async fn play_shows_the_raw_resume_digest(world: &mut World) {
    wait_play(world, "\"kind\":\"view\"").await;
    let stdout = capture_play_stdout(world);
    let resume = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .map(|message| message["body"].clone())
        .find(|body| body["kind"] == "resume")
        .unwrap_or_else(|| panic!("no resume frame\n{stdout}"));
    assert_eq!(resume["notes"][0]["id"], "note_mira_door", "{stdout}");
    assert_eq!(
        resume["notes"][0]["text"], "Back at dusk. Oil on the shelf.",
        "{stdout}"
    );
    assert!(!stdout.contains("While you were away"), "{stdout}");
}

#[then(expr = "stderr shows {string}")]
fn stderr_shows(world: &mut World, text: String) {
    assert_eq!(world.stderr.trim(), text, "{}", world.stderr);
    assert_eq!(
        world.stderr.lines().count(),
        text.lines().count(),
        "{}",
        world.stderr
    );
}

#[then(expr = "stdout shows the picker:")]
fn stdout_shows_the_picker(world: &mut World, step: &Step) {
    let picker = step.docstring.as_deref().unwrap_or_default().trim();
    assert!(world.stdout.contains(picker), "{}", world.stdout);
}

#[then(expr = "stderr has no protocol JSON")]
fn stderr_has_no_protocol_json(world: &mut World) {
    assert!(!world.stderr.contains('{'), "{}", world.stderr);
    assert!(!world.stderr.contains("\"kind\""), "{}", world.stderr);
}

#[given(expr = "a docks table where harbormouse's might is 40")]
fn harbormouses_might_is_40(world: &mut World) {
    a_docks_table(world);
    edit_save(host::pack(world), |now| {
        let might = Meter {
            base: 40,
            offset: 0,
        };
        now.actors_mut()
            .get_mut::<Meter>(&"harbormouse".into())
            .set("might", Some(might))
            .unwrap();
    });
}

#[given(expr = "a docks table where hp has no name")]
fn hp_has_no_name(world: &mut World) {
    a_docks_table(world);
    let meters = host::pack(world).join("rules/meters.yaml");
    replace_in_file(&meters, "    name: Health\n", "");
    spookery_engine::migrate_pack(host::pack(world)).unwrap_or_else(|err| panic!("{err}"));
}

#[given(expr = "a docks table where a spill leaves Briar a note and costs Briar 1 hp")]
fn a_spill_leaves_briar_a_note(world: &mut World) {
    a_docks_table(world);
    let pack = host::pack(world).to_path_buf();
    add_verb(
        &pack,
        "  - id: spill\n    when:\n      verb: spill\n    do:\n      - note:\n          id: note_sealed_name\n          to_ids:\n            - briar\n          text: the sealed name\n      - flag:\n          of: world\n          key: sealed_name_spilled\n          value: true\n      - meter:\n          of: briar\n          key: hp\n          add: -1\n",
    );
    stamp_pack_rev(&pack);
}

#[given(expr = "a docks table where a sip costs harbormouse a tonic and gives Bram one")]
fn a_sip_gives_bram_a_tonic(world: &mut World) {
    a_docks_table(world);
    let pack = host::pack(world).to_path_buf();
    add_verb(
        &pack,
        "  - id: sip\n    when:\n      verb: sip\n    cost:\n      items:\n        tonic: 1\n    do:\n      - item:\n          of: bram\n          id: tonic\n          add: 1\n",
    );
    stamp_pack_rev(&pack);
}

#[given(expr = "a docks table where a juggle gains harbormouse a tonic and gives it back")]
fn a_juggle_gives_the_tonic_back(world: &mut World) {
    a_docks_table(world);
    let pack = host::pack(world).to_path_buf();
    add_verb(
        &pack,
        "  - id: juggle\n    when:\n      verb: juggle\n    do:\n      - item:\n          of: harbormouse\n          id: tonic\n          add: 1\n      - item:\n          of: harbormouse\n          id: tonic\n          add: -1\n",
    );
    stamp_pack_rev(&pack);
}
