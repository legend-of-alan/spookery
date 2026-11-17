use std::collections::BTreeSet;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;

use cucumber::{then, when};

use crate::support::{self, copy_pack};
use crate::World;

use super::table;

/// The table starts in a directory of its own, where it writes its log file.
#[when(expr = "the table listens on port 0")]
fn the_table_listens_on_port_0(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let dir = support::fresh_dir("host-start");
    table::run_with_flags(world, &dir, &pack, &["--addr", "127.0.0.1:0"]);
    world.scratch = dir.to_string_lossy().into_owned();
}

#[then(
    expr = "the host names its log file in the directory it started in, and the file says the pack opened"
)]
fn the_host_names_its_log_file(world: &mut World) {
    let stderr = table::poll_until("the log file's path", || {
        let stderr = table::host_stderr(world);
        if stderr.ends_with('\n') {
            Ok(stderr)
        } else {
            Err(stderr)
        }
    });
    let named = stderr
        .strip_prefix("logging to ")
        .and_then(|rest| rest.strip_suffix('\n'))
        .unwrap_or_else(|| panic!("no log path on stderr:\n{stderr}"));
    let file = fs::canonicalize(named).unwrap_or_else(|err| panic!("{named}: {err}"));
    let logs = fs::canonicalize(Path::new(&world.scratch).join("logs"))
        .unwrap_or_else(|err| panic!("no logs/ where the host started: {err}"));
    assert_eq!(file.parent(), Some(logs.as_path()), "{stderr}");
    let file_name = file.file_name().unwrap_or_default().to_string_lossy();
    assert!(
        file_name.starts_with("run-") && file_name.ends_with(".log"),
        "{stderr}"
    );
    let text = fs::read_to_string(&file).unwrap_or_else(|err| panic!("{named}: {err}"));
    assert!(text.contains("pack opened"), "{text}");
}

#[then(expr = "the pack holds no log file")]
fn the_pack_holds_no_log_file(world: &mut World) {
    for (path, _) in table::read_files(table::pack(world)) {
        let is_log = path.extension().is_some_and(|extension| extension == "log");
        assert!(!is_log, "{}", path.display());
    }
}

#[then(expr = "the host reports the game port and the MCP address")]
fn the_host_reports_the_game_port_and_the_mcp_addr(world: &mut World) {
    let live = table::live_mut(world);
    let stdout = live
        .host
        .as_ref()
        .map(|server| support::text_of(&server.stdout))
        .unwrap_or_default();
    let game = live.game_port;
    let mcp_port = live.mcp_port;
    let (reported_game, reported_mcp_addr) = table::parse_addrs(&stdout);
    assert_eq!(reported_game.port(), game, "{stdout}");
    assert_eq!(reported_mcp_addr.port(), mcp_port, "{stdout}");
    assert_ne!(game, mcp_port, "{stdout}");
    assert_ne!(game, 0, "{stdout}");
    assert_ne!(mcp_port, 0, "{stdout}");
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.contains("\"kind\":\"listening\""))
            .count(),
        1,
        "{stdout}"
    );
    TcpStream::connect(("127.0.0.1", game)).unwrap_or_else(|err| panic!("game: {err}"));
    TcpStream::connect(("127.0.0.1", mcp_port)).unwrap_or_else(|err| panic!("mcp: {err}"));
}

#[when(expr = "a player connects to the reported game port")]
fn a_player_connects_to_the_reported_game_port(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::seat_rowan(world, &pack);
}

#[then(expr = "the player receives a perceive line")]
fn the_player_receives_a_perceive_line(world: &mut World) {
    let text = table::player_text(world);
    assert!(text.contains("\"kind\":\"view\""), "{text}");
}

#[when(expr = "two tables listen on port 0 at the same time")]
fn two_tables_listen_on_port_0_at_the_same_time(world: &mut World) {
    let first = table::pack(world).to_path_buf();
    let second = copy_pack("docks", "ports-other");
    world.scratch = second.to_string_lossy().into_owned();
    let (dir, flags) = (std::env::temp_dir(), ["--addr", "127.0.0.1:0"]);
    let started_first = table::spawn(&dir, &first, &flags);
    let started_second = table::spawn(&dir, &second, &flags);
    let first = table::wait_addrs(started_first);
    let second = table::wait_addrs(started_second);
    world.stdout = format!(
        "{} {} {} {}",
        first.game, first.mcp_addr, second.game, second.mcp_addr
    );
    world.plays.push(first.host);
    world.plays.push(second.host);
}

#[then(expr = "the tables report distinct ports")]
fn the_tables_report_distinct_ports(world: &mut World) {
    // repeats module: mirrors its step text
    let ports: Vec<SocketAddr> = world
        .stdout
        .split_whitespace()
        .map(|text| text.parse().unwrap_or_else(|err| panic!("{err}: {text}")))
        .collect();
    assert_eq!(ports.len(), 4, "{}", world.stdout);
    let distinct: BTreeSet<u16> = ports.iter().map(|addr| addr.port()).collect();
    assert_eq!(distinct.len(), 4, "{}", world.stdout);
    for addr in &ports {
        TcpStream::connect(*addr).unwrap_or_else(|err| panic!("connect {addr}: {err}"));
    }
    world.plays.clear();
    let _ = fs::remove_dir_all(&world.scratch);
}
