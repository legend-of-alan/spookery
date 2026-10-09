use std::io::Write;
use std::time::{Duration, Instant};

use cucumber::{then, when};

use crate::support;
use crate::World;

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

#[then(expr = "the command exits 2")]
fn the_command_exits_2(world: &mut World) {
    assert_eq!(
        world.code,
        Some(2),
        "stdout:\n{}\nstderr:\n{}",
        world.stdout,
        world.stderr
    );
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

#[when(expr = "play joins the table on a free loopback address")]
fn play_joins_the_table_on_a_free_loopback_address(world: &mut World) {
    join_play(world, false);
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
    let port = crate::with_live(|live| live.port);
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
    assert!(stdout.contains("Docks shop"), "{stdout}");
    assert!(!stdout.contains("\"kind\":\"perceive\""), "{stdout}");
}

#[then(expr = "play shows {string}")]
fn play_shows(world: &mut World, text: String) {
    wait_play(world, &text);
}

#[then(expr = "play shows no protocol JSON")]
fn play_shows_no_protocol_json(world: &mut World) {
    let stdout = play_stdout(world);
    assert!(!stdout.contains('{'), "{stdout}");
    assert!(!stdout.contains("\"kind\""), "{stdout}");
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

fn join_play(world: &mut World, raw: bool) {
    let port = table::free_port();
    let addr = format!("127.0.0.1:{port}");
    let pack = table::pack(world);
    table::listen(pack, &addr, port);
    let pack_arg = support::pack_arg(pack);
    let mut args = vec!["play".to_string(), pack_arg, "--addr".to_string(), addr];
    if raw {
        args.push("--raw".to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    world.play = Some(support::spawn(&refs));
    let needle = if raw {
        "\"kind\":\"perceive\""
    } else {
        "Docks shop"
    };
    wait_play(world, needle);
}

fn wait_play(world: &mut World, needle: &str) {
    let start = Instant::now();
    loop {
        let stdout = play_stdout(world);
        if stdout.contains(needle) {
            return;
        }
        let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
        if let Some(status) = play
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            let stderr = support::text_of(&play.stderr);
            panic!("play exited {status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        if start.elapsed() > Duration::from_secs(8) {
            let stderr = world
                .play
                .as_ref()
                .map(|play| support::text_of(&play.stderr))
                .unwrap_or_default();
            panic!("missing {needle}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn play_stdout(world: &mut World) -> String {
    let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
    let text = support::text_of(&play.stdout);
    world.stdout = text.clone();
    text
}
