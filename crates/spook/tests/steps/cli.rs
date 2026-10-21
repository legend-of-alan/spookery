use std::io::Write;
use std::time::{Duration, Instant};

use cucumber::{then, when};

use crate::support;
use crate::World;

use super::common::{join_play, play_stdout, wait_play, wait_play_until};
use super::table;

#[when(expr = "the operator asks spook for help")]
fn the_operator_asks_spook_for_help(world: &mut World) {
    table::run(world, &["--help"]);
}

#[when(expr = "the operator asks the {string} command for help")]
fn the_operator_asks_a_command_for_help(world: &mut World, command: String) {
    table::run(world, &[&command, "--help"]);
}

#[when(expr = "the operator runs the arguments {string} and {string}")]
fn the_operator_runs_two_arguments(world: &mut World, first: String, second: String) {
    table::run(world, &[&first, &second]);
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

#[when(expr = "play joins the table with raw frames")]
fn play_joins_the_table_with_raw_frames(world: &mut World) {
    join_play(world, true);
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

#[when(expr = "another play joins that table")]
fn another_play_joins_that_table(world: &mut World) {
    let port = table::live_mut(world).port;
    let addr = format!("127.0.0.1:{port}");
    let pack = support::pack_arg(table::pack(world));
    let output = support::spook(&["play", &pack, "--addr", &addr]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[then(expr = "play received the first perceive")]
fn play_received_the_first_perceive(world: &mut World) {
    let stdout = play_stdout(world);
    assert!(stdout.contains("Docks Shop"), "{stdout}");
    assert!(!stdout.contains("\"kind\":\"perceive\""), "{stdout}");
}

#[then(expr = "play does not show {string}")]
fn play_does_not_show(world: &mut World, text: String) {
    let stdout = play_stdout(world);
    assert!(!stdout.contains(&text), "{stdout}");
}

#[then(expr = "play does not show the line {string}")]
fn play_does_not_show_the_line(world: &mut World, text: String) {
    let stdout = play_stdout(world);
    assert!(stdout.lines().all(|line| line != text), "{stdout}");
}

#[then(expr = "play's error shows {string}")]
fn play_error_shows(world: &mut World, text: String) {
    let start = Instant::now();
    loop {
        let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
        let stderr = support::text_of(&play.stderr);
        if stderr.contains(&text) {
            world.stderr = stderr;
            return;
        }
        if start.elapsed() > Duration::from_secs(8) {
            let stdout = play_stdout(world);
            panic!("missing {text}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[then(expr = "play shows protocol frames")]
fn play_shows_protocol_frames(world: &mut World) {
    wait_play(world, "\"kind\":\"perceive\"");
    let stdout = play_stdout(world);
    assert!(stdout.contains("\"kind\":\"resume\""), "{stdout}");
    assert!(stdout.contains('{'), "{stdout}");
}

#[then(expr = "play shows the door note and the debt clock as sentences")]
fn play_shows_the_readable_resume(world: &mut World) {
    let away = "\
While you were away:
Back at dusk. Oil on the shelf.
Assassination debt: 2/6, paused
";
    wait_play(world, away);
    let stdout = play_stdout(world);
    assert!(!stdout.contains("note_mira_door"), "{stdout}");
    assert!(!stdout.contains("clock assassination_debt"), "{stdout}");
    assert!(!stdout.contains("message "), "{stdout}");
}

#[then(expr = "the round shows no away heading")]
fn the_round_shows_no_away_heading(world: &mut World) {
    wait_play_until(
        world,
        |stdout| stdout.matches("You are in").count() >= 2,
        "a second You are in",
    );
    let stdout = play_stdout(world);
    let after = stdout.split_once("You are in").unwrap().1;
    assert!(!after.contains("While you were away:"), "{stdout}");
}

#[then(expr = "the round's update shows the door note only under Notes")]
fn the_rounds_update_shows_the_door_note_only_under_notes(world: &mut World) {
    let note = "Back at dusk. Oil on the shelf.";
    wait_play_until(
        world,
        |stdout| stdout.matches("You are in").count() >= 2,
        "a second You are in",
    );
    let stdout = play_stdout(world);
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
    let stdout = play_stdout(world);
    let earlier = "\
Earlier:
You: hello Briar

";
    assert!(stdout.starts_with(earlier), "{stdout}");
    assert_eq!(stdout.matches("Earlier:").count(), 1, "{stdout}");
    assert_eq!(stdout.matches("You: hello Briar").count(), 1, "{stdout}");
    let live = &stdout[earlier.len()..];
    assert!(live.starts_with("While you were away:\n"), "{stdout}");
}

#[then(expr = "play shows the raw resume digest")]
fn play_shows_the_raw_resume_digest(world: &mut World) {
    wait_play(world, "\"kind\":\"perceive\"");
    let stdout = play_stdout(world);
    let resume = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|frame| frame["kind"] == "resume")
        .unwrap_or_else(|| panic!("no resume frame\n{stdout}"));
    assert_eq!(resume["text"], super::digest::RESUME, "{stdout}");
    assert!(!stdout.contains("While you were away"), "{stdout}");
}

#[then(expr = "stderr shows {string}")]
fn stderr_shows(world: &mut World, text: String) {
    assert_eq!(world.stderr.trim(), text, "{}", world.stderr);
}

#[then(expr = "stderr has no protocol JSON")]
fn stderr_has_no_protocol_json(world: &mut World) {
    assert!(!world.stderr.contains('{'), "{}", world.stderr);
    assert!(!world.stderr.contains("\"kind\""), "{}", world.stderr);
}
