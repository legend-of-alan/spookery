use std::io::Write;
use std::time::{Duration, Instant};

use cucumber::{then, when};

use crate::support::{self, Proc};
use crate::World;

use super::table;

#[when(expr = "play and a human GM share a free loopback port")]
fn play_and_a_human_gm_share_a_port(world: &mut World) {
    let addr = sit_gm(world, false);
    sit_play(world, &addr, None);
}

#[when(expr = "two players and a human GM share a free loopback port")]
fn two_players_and_a_human_gm_share_a_port(world: &mut World) {
    let addr = sit_gm(world, false);
    sit_play(world, &addr, Some("pc_rowan"));
    sit_play(world, &addr, Some("pc_wren"));
}

#[when(expr = "a human GM sits on a free loopback port")]
fn a_human_gm_sits(world: &mut World) {
    let _ = sit_gm(world, false);
}

#[when(expr = "a human GM sits with raw frames on a free loopback port")]
fn a_human_gm_sits_raw(world: &mut World) {
    let _ = sit_gm(world, true);
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

#[when(expr = "the GM queues Mira's restock and commits")]
fn the_gm_queues_restock_and_commits(world: &mut World) {
    write_gm(world, "queue mira restock_lamp_oil");
    write_gm(world, "commit");
}

#[when(expr = "the GM queues Rowan's grit and commits")]
fn the_gm_queues_grit_and_commits(world: &mut World) {
    let before = count_gm(world, "Queue:");
    write_gm(world, "queue pc_rowan grit");
    write_gm(world, "commit");
    // The status result follows every check event of that commit on the
    // socket, so its Queue line means the client has read them all.
    write_gm(world, "status");
    wait_gm_count(world, "Queue:", before + 1);
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
fn another_gm_joins(world: &mut World) {
    let port = crate::with_live(|live| live.port);
    let addr = format!("127.0.0.1:{port}");
    let pack = support::pack_arg(table::pack(world));
    let output = support::spook(&["gm", &pack, "--addr", &addr]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[when(expr = "the GM queues an unknown move")]
fn the_gm_queues_an_unknown_move(world: &mut World) {
    write_gm(world, "queue mira not_a_move");
}

#[when(expr = "spook run stops")]
fn spook_run_stops(_world: &mut World) {
    crate::with_live(|live| {
        live.server.take();
    });
}

#[then(expr = "the GM exits")]
fn the_gm_exits(world: &mut World) {
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    let start = Instant::now();
    loop {
        if let Some(status) = gm.child.try_wait().unwrap_or_else(|err| panic!("{err}")) {
            assert!(status.success(), "gm exited {status:?}");
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("gm still running");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[when(expr = "spook run listens with an overridden agent door")]
fn spook_run_listens_with_an_overridden_agent_door(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let _ = table::listen_door(&pack);
}

#[when(expr = "a human GM dials that door")]
fn a_human_gm_dials_that_door(world: &mut World) {
    let (game, http) = crate::with_live(|live| (live.port, live.http_port));
    let pack_arg = support::pack_arg(table::pack(world));
    let game = format!("127.0.0.1:{game}");
    let http = format!("127.0.0.1:{http}");
    world.gm = Some(support::spawn(&[
        "gm",
        &pack_arg,
        "--addr",
        &game,
        "--mcp-http",
        &http,
    ]));
    wait_gm(world, "Commands:");
}

#[when(expr = "the operator runs gm with nothing listening")]
fn gm_with_nothing_listening(world: &mut World) {
    let port = table::free_port();
    assert_ne!(port, 9737);
    let addr = format!("127.0.0.1:{port}");
    let pack = support::pack_arg(table::pack(world));
    let output = support::spook(&["gm", &pack, "--addr", &addr]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[then(expr = "the GM shows {string}")]
fn the_gm_shows(world: &mut World, text: String) {
    wait_gm(world, &text);
}

#[then(expr = "the GM shows {int} rolls")]
fn the_gm_shows_rolls(world: &mut World, rolls: usize) {
    let stdout = gm_stdout(world);
    let shown = stdout.matches("Roll: ").count();
    assert_eq!(shown, rolls, "{stdout}");
}

#[then(expr = "the GM shows protocol JSON")]
fn the_gm_shows_protocol_json(world: &mut World) {
    let stdout = gm_stdout(world);
    assert!(stdout.contains("{\"jsonrpc\""), "{stdout}");
}

#[then(expr = "the GM shows no protocol JSON")]
fn the_gm_shows_no_protocol_json(world: &mut World) {
    let stdout = gm_stdout(world);
    assert!(!stdout.contains("{\"jsonrpc\""), "{stdout}");
}

#[then(expr = "world time advanced one live tick")]
fn world_time_advanced(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let start = Instant::now();
    loop {
        let now = table::now_of(&pack);
        if now.world_time == 1085 {
            return;
        }
        if start.elapsed() > Duration::from_secs(8) {
            panic!("world_time {}", now.world_time);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[then(expr = "the tick log records Mira's restock")]
fn the_tick_log_records_the_restock(world: &mut World) {
    let tick = tick_json(world);
    let intents = tick["intents"]
        .as_array()
        .unwrap_or_else(|| panic!("{tick}"));
    assert!(
        intents
            .iter()
            .any(|intent| { intent["move"] == "restock_lamp_oil" && intent["actor_id"] == "mira" }),
        "{tick}"
    );
}

#[then(expr = "the tick log records Rowan and Wren waiting")]
fn the_tick_log_records_rowan_and_wren(world: &mut World) {
    let start = Instant::now();
    loop {
        if table::tick_names(table::pack(world)).is_empty() {
            if start.elapsed() > Duration::from_secs(8) {
                panic!("no tick log");
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        break;
    }
    let tick = tick_json(world);
    let waits = tick["intents"]
        .as_array()
        .unwrap_or_else(|| panic!("{tick}"))
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .map(|intent| {
            intent["actor_id"]
                .as_str()
                .unwrap_or_else(|| panic!("actor"))
                .to_string()
        })
        .collect::<Vec<_>>();
    assert!(waits.iter().any(|id| id == "pc_rowan"), "{waits:?}");
    assert!(waits.iter().any(|id| id == "pc_wren"), "{waits:?}");
    assert_eq!(waits.len(), 2, "{waits:?}");
}

#[then(expr = "the GM's stderr shows {string}")]
fn the_gm_stderr_shows(world: &mut World, text: String) {
    let start = Instant::now();
    loop {
        let stderr = gm_stderr(world);
        if stderr.contains(&text) {
            return;
        }
        let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
        if let Some(status) = gm.child.try_wait().unwrap_or_else(|err| panic!("{err}")) {
            panic!("gm exited {status:?}\nstderr:\n{stderr}");
        }
        if start.elapsed() > Duration::from_secs(8) {
            panic!("missing {text}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn sit_gm(world: &mut World, raw: bool) -> String {
    let port = table::free_port();
    assert_ne!(port, 9737);
    let addr = format!("127.0.0.1:{port}");
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack, &addr, port);
    let pack_arg = support::pack_arg(&pack);
    let mut args = vec![
        "gm".to_string(),
        pack_arg,
        "--addr".to_string(),
        addr.clone(),
    ];
    if raw {
        args.push("--raw".to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    world.gm = Some(support::spawn(&refs));
    let needle = if raw { "{\"jsonrpc\"" } else { "Commands:" };
    wait_gm(world, needle);
    addr
}

fn sit_play(world: &mut World, addr: &str, as_player: Option<&str>) {
    let pack_arg = support::pack_arg(table::pack(world));
    let mut args = vec![
        "play".to_string(),
        pack_arg,
        "--addr".to_string(),
        addr.to_string(),
    ];
    if let Some(id) = as_player {
        args.push("--as".to_string());
        args.push(format!("player:{id}"));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut play = support::spawn(&refs);
    wait_proc(&mut play, "You are in");
    if world.play.is_none() {
        world.play = Some(play);
    } else {
        world.plays.push(play);
    }
}

fn write_play(play: &mut Proc, line: &str) {
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

fn wait_gm(world: &mut World, needle: &str) {
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    wait_proc(gm, needle);
}

fn count_gm(world: &mut World, needle: &str) -> usize {
    gm_stdout(world).matches(needle).count()
}

fn wait_gm_count(world: &mut World, needle: &str, count: usize) {
    let start = Instant::now();
    loop {
        let stdout = gm_stdout(world);
        if stdout.matches(needle).count() >= count {
            return;
        }
        if start.elapsed() > Duration::from_secs(8) {
            let stderr = gm_stderr(world);
            panic!("missing {count} x {needle}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_proc(proc: &mut Proc, needle: &str) {
    let start = Instant::now();
    loop {
        let stdout = support::text_of(&proc.stdout);
        if stdout.contains(needle) {
            return;
        }
        if let Some(status) = proc.child.try_wait().unwrap_or_else(|err| panic!("{err}")) {
            let stderr = support::text_of(&proc.stderr);
            panic!("exited {status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        if start.elapsed() > Duration::from_secs(8) {
            let stderr = support::text_of(&proc.stderr);
            panic!("missing {needle}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn gm_stdout(world: &mut World) -> String {
    let gm = world.gm.as_ref().unwrap_or_else(|| panic!("no gm"));
    let text = support::text_of(&gm.stdout);
    world.stdout = text.clone();
    text
}

fn gm_stderr(world: &mut World) -> String {
    let gm = world.gm.as_ref().unwrap_or_else(|| panic!("no gm"));
    support::text_of(&gm.stderr)
}

fn tick_json(world: &mut World) -> serde_json::Value {
    let text = std::fs::read_to_string(table::pack(world).join("state/journal/tick-0001.jsonl"))
        .unwrap_or_else(|err| panic!("read tick: {err}"));
    serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("{err}"))
}
