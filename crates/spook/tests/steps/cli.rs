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
    let port = table::free_port();
    let addr = format!("127.0.0.1:{port}");
    let pack = table::pack(world);
    table::listen(pack, &addr, port);
    let pack_arg = support::pack_arg(pack);
    let mut play = support::spawn(&["play", &pack_arg, "--addr", &addr]);
    let start = Instant::now();
    loop {
        let text = support::text_of(&play.stdout);
        if text.contains("\"kind\":\"perceive\"") {
            world.stdout = text;
            return;
        }
        if let Some(status) = play
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            panic!(
                "play exited {status:?}\nstdout:\n{}\nstderr:\n{}",
                support::text_of(&play.stdout),
                support::text_of(&play.stderr)
            );
        }
        if start.elapsed() > Duration::from_secs(8) {
            panic!(
                "play did not perceive\nstdout:\n{}\nstderr:\n{}",
                support::text_of(&play.stdout),
                support::text_of(&play.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[then(expr = "play received the first perceive")]
fn play_received_the_first_perceive(world: &mut World) {
    assert!(
        world.stdout.contains("\"kind\":\"perceive\""),
        "{}",
        world.stdout
    );
}
