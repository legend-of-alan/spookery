use spookery_engine::testing::world_of;
use std::io::Write;

use cucumber::gherkin::Step;
use cucumber::{given, then, when};

use crate::support::{self, Process};
use crate::World;

use super::{common, host};

/// The start of the hello as raw mode prints it.
const HELLO_JSON: &str = r#"{"body":{"kind":"hello""#;

#[when(expr = "play and a human GM share a free loopback port")]
async fn play_and_a_human_gm_share_a_port(world: &mut World) {
    let addr = connect_gm(world, false).await;
    play_as_pc_at(world, &addr, None).await;
}

#[when(expr = "two players and a human GM share a free loopback port")]
async fn two_players_and_a_human_gm_share_a_port(world: &mut World) {
    let addr = connect_gm(world, false).await;
    play_as_pc_at(world, &addr, Some("rowan")).await;
    play_as_pc_at(world, &addr, Some("wren")).await;
}

#[when(expr = "a human GM connects on a free loopback port")]
async fn a_human_gm_connects(world: &mut World) {
    let _ = connect_gm(world, false).await;
}

#[when(expr = "a human GM connects with raw messages on a free loopback port")]
async fn a_human_gm_connects_raw(world: &mut World) {
    let _ = connect_gm(world, true).await;
}

#[when(expr = "the player says hello to the GM")]
fn the_player_says_hello(world: &mut World) {
    write_play(
        world.play.as_mut().unwrap_or_else(|| panic!("no play")),
        "say hello",
    );
}

#[when(expr = "the GM says {string}")]
fn the_gm_says(world: &mut World, text: String) {
    write_gm(world, &format!("say {text}"));
}

#[when(expr = "the GM queues Mira's door note and commits")]
fn the_gm_queues_the_door_note_and_commits(world: &mut World) {
    write_gm(world, "queue mira leave_door_note");
    write_gm(world, "commit");
}

#[given(expr = "a docks table with a GM and {string} seated")]
async fn a_docks_table_with_a_gm_and_a_seat(world: &mut World, actor: String) {
    let pack = support::copy_pack("docks", "typed-gm");
    world.pack = Some(pack.clone());
    host::listen(world, &pack).await;
    host::seat_actor(world, &pack, &actor).await;
    let addr = host::game_addr(world);
    world.gm = Some(support::spawn(&["gm", "--addr", &addr]));
    wait_gm(world, "Commands:").await;
}

#[when(expr = "the GM queues {string} for {string} and commits")]
fn the_gm_queues_for_and_commits(world: &mut World, verb_id: String, actor: String) {
    write_gm(world, &format!("queue {actor} {verb_id}"));
    write_gm(world, "commit");
}

#[then(expr = "the GM terminal shows one check line and one dice line for {string}")]
async fn the_gm_terminal_shows_one_check_and_one_dice(world: &mut World, shown: String) {
    let queued = format!("{shown} queued:");
    wait_gm(world, "Roll:").await;
    let stdout = capture_gm_stdout(world);
    let checks = stdout
        .lines()
        .filter(|line| line.starts_with("Roll:"))
        .count();
    let dice = stdout
        .lines()
        .filter(|line| line.starts_with("dice "))
        .count();
    assert_eq!(checks, 1, "{stdout}");
    assert_eq!(dice, 1, "{stdout}");
    assert!(stdout.contains(&queued), "{stdout}");
}

/// Play holds Rowan, so the GM drives Wren.
#[when(expr = "the GM queues Wren's exert and commits")]
async fn the_gm_queues_exert_and_commits(world: &mut World) {
    let before = count_gm_matches(world, "Queue:");
    write_gm(world, "queue wren exert");
    write_gm(world, "commit");
    write_gm(world, "status");
    wait_gm_count(world, "Queue:", before + 1).await;
}

#[when(expr = "each player queues a wait")]
fn each_player_queues_a_wait(world: &mut World) {
    write_play(
        world.play.as_mut().unwrap_or_else(|| panic!("no play")),
        "wait",
    );
    let extra = world
        .plays
        .first_mut()
        .unwrap_or_else(|| panic!("no second play"));
    write_play(extra, "wait");
}

#[when(expr = "the GM commits the queue")]
fn the_gm_commits(world: &mut World) {
    write_gm(world, "commit");
}

#[when(expr = "another GM joins that seat")]
async fn another_gm_joins(world: &mut World) {
    let addr = host::game_addr(world);
    host::run(world, &["gm", "--addr", &addr]).await;
}

#[when(expr = "the GM is killed")]
async fn the_gm_is_killed(world: &mut World) {
    let mark = host::stdout(world).len();
    world.gm.take();
    host::wait_until(world, "the GM seat freed", |text| {
        text.get(mark..)
            .is_some_and(|added| added.contains("gm seat free"))
    })
    .await;
}

#[when(expr = "another GM connects to that seat")]
fn another_gm_connects_to_that_seat(world: &mut World) {
    let addr = host::game_addr(world);
    world.gm = Some(support::spawn(&["gm", "--addr", &addr]));
}

#[when(expr = "the GM types {string}")]
fn the_gm_types(world: &mut World, line: String) {
    write_gm(world, &line);
}

#[when(expr = "the GM queues an unknown verb")]
fn the_gm_queues_an_unknown_verb(world: &mut World) {
    write_gm(world, "queue mira not_a_verb");
}

#[when(expr = "spook run stops")]
fn spook_run_stops(world: &mut World) {
    host::live_mut(world).process.take();
}

#[then(expr = "the GM exits")]
async fn the_gm_exits(world: &mut World) {
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    let status = host::poll_until("the gm to exit", async || {
        let status = gm.child.try_wait().unwrap_or_else(|err| panic!("{err}"));
        status.ok_or("still running")
    })
    .await;
    let stderr = support::text_of(&gm.stderr);
    assert!(status.success(), "gm exited {status:?}\n{stderr}");
}

#[when(expr = "the operator runs gm with nothing listening")]
async fn gm_with_nothing_listening(world: &mut World) {
    host::run(world, &["gm", "--addr", "127.0.0.1:1"]).await;
}

#[then(expr = "the GM shows {string}")]
async fn the_gm_shows(world: &mut World, text: String) {
    wait_gm(world, &text).await;
}

#[then(expr = "the GM shows each of these blocks:")]
async fn the_gm_shows_each_block(world: &mut World, step: &Step) {
    for block in super::cli::split_blocks(step) {
        wait_gm(world, block).await;
    }
}

#[then(expr = "the GM does not show {string}")]
fn the_gm_does_not_show(world: &mut World, text: String) {
    let stdout = capture_gm_stdout(world);
    assert!(!stdout.contains(&text), "{stdout}");
}

#[then(expr = "the GM shows {int} rolls")]
fn the_gm_shows_rolls(world: &mut World, rolls: usize) {
    let stdout = capture_gm_stdout(world);
    let shown = stdout.matches("Roll: ").count();
    assert_eq!(shown, rolls, "{stdout}");
}

#[then(expr = "the GM shows protocol JSON")]
fn the_gm_shows_protocol_json(world: &mut World) {
    let stdout = capture_gm_stdout(world);
    assert!(stdout.contains(HELLO_JSON), "{stdout}");
}

#[then(expr = "the GM shows no protocol JSON")]
fn the_gm_shows_no_protocol_json(world: &mut World) {
    let stdout = capture_gm_stdout(world);
    assert!(!stdout.contains("\"kind\":"), "{stdout}");
}

#[then(expr = "world time advanced one live tick")]
async fn world_time_advanced(world: &mut World) {
    let pack = host::pack(world).to_path_buf();
    host::poll_until("world_time 1085", async || {
        match world_of(&pack).world_time {
            1085 => Ok(()),
            now => Err(now),
        }
    })
    .await;
}

#[then(expr = "the GM's stderr shows {string}")]
async fn the_gm_stderr_shows(world: &mut World, text: String) {
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    host::poll_until(&text, async || {
        let stderr = support::text_of(&gm.stderr);
        if stderr.contains(&text) {
            return Ok(());
        }
        if let Some(status) = gm.child.try_wait().unwrap_or_else(|err| panic!("{err}")) {
            panic!("gm exited {status:?}\nstderr:\n{stderr}");
        }
        Err(stderr)
    })
    .await;
}

async fn connect_gm(world: &mut World, raw: bool) -> String {
    let pack = host::pack(world).to_path_buf();
    host::listen(world, &pack).await;
    let addr = host::game_addr(world);
    let mut args = vec!["gm".to_string(), "--addr".to_string(), addr.clone()];
    if raw {
        args.push("--raw".to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    world.gm = Some(support::spawn(&refs));
    let needle = if raw { HELLO_JSON } else { "Commands:" };
    wait_gm(world, needle).await;
    addr
}

async fn play_as_pc_at(world: &mut World, addr: &str, as_player: Option<&str>) {
    let mut play = common::spawn_play(addr, as_player.unwrap_or("rowan"), &[]);
    common::wait_proc_until(
        &mut play,
        |stdout| stdout.contains("You are in"),
        "You are in",
    )
    .await;
    if world.play.is_none() {
        world.play = Some(play);
    } else {
        world.plays.push(play);
    }
}

fn write_play(play: &mut Process, line: &str) {
    let stdin = play.stdin.as_mut().unwrap_or_else(|| panic!("play stdin"));
    writeln!(stdin, "{line}").unwrap_or_else(|err| panic!("write play: {err}"));
    stdin
        .flush()
        .unwrap_or_else(|err| panic!("flush play: {err}"));
}

fn write_gm(world: &mut World, line: &str) {
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    let stdin = gm.stdin.as_mut().unwrap_or_else(|| panic!("gm stdin"));
    writeln!(stdin, "{line}").unwrap_or_else(|err| panic!("write gm: {err}"));
    stdin
        .flush()
        .unwrap_or_else(|err| panic!("flush gm: {err}"));
}

async fn wait_gm(world: &mut World, needle: &str) {
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    common::wait_proc_until(gm, |stdout| stdout.contains(needle), needle).await;
}

fn count_gm_matches(world: &mut World, needle: &str) -> usize {
    capture_gm_stdout(world).matches(needle).count()
}

async fn wait_gm_count(world: &mut World, needle: &str, count: usize) {
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    let enough = |stdout: &str| stdout.matches(needle).count() >= count;
    world.stdout = common::wait_proc_until(gm, enough, &format!("{count} x {needle}")).await;
}

fn capture_gm_stdout(world: &mut World) -> String {
    let gm = world.gm.as_ref().unwrap_or_else(|| panic!("no gm"));
    let text = support::text_of(&gm.stdout);
    world.stdout = text.clone();
    text
}
