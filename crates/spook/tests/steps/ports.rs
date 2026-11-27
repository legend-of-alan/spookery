use std::collections::BTreeSet;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cucumber::{then, when};

use crate::support::{self, copy_pack};
use crate::World;

use super::host;

/// The host starts in a directory of its own, where it writes its log file.
#[when(expr = "the table listens on port 0")]
async fn the_table_listens_on_port_0(world: &mut World) {
    let pack = host::pack(world).to_path_buf();
    let dir = support::fresh_dir("host-start");
    host::run_with_flags(world, &dir, &pack, &["--addr", "127.0.0.1:0"]).await;
    world.scratch = dir.to_string_lossy().into_owned();
}

#[then(
    expr = "the host names its log file in the directory it started in, and the file says the pack opened"
)]
async fn the_host_names_its_log_file(world: &mut World) {
    let host = host::live(world).process.as_ref();
    let host = host.unwrap_or_else(|| panic!("no host"));
    let file = wait_for_log_file(&host.stderr).await;
    let logs = fs::canonicalize(Path::new(&world.scratch).join("logs"))
        .unwrap_or_else(|err| panic!("no logs/ where the host started: {err}"));
    assert_eq!(file.parent(), Some(logs.as_path()), "{}", file.display());
    let file_name = file.file_name().unwrap_or_default().to_string_lossy();
    assert!(
        file_name.starts_with("run-") && file_name.ends_with(".log"),
        "{file_name}"
    );
    let text = fs::read_to_string(&file).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
    assert!(text.contains("pack opened"), "{text}");
}

/// Waits for the host's `logging to <path>` line on stderr and returns that file.
async fn wait_for_log_file(stderr: &Arc<Mutex<String>>) -> PathBuf {
    let stderr = host::poll_until("the log file's path", async || {
        let stderr = support::text_of(stderr);
        if stderr.ends_with('\n') {
            Ok(stderr)
        } else {
            Err(stderr)
        }
    })
    .await;
    let named = stderr
        .strip_prefix("logging to ")
        .and_then(|rest| rest.strip_suffix('\n'))
        .unwrap_or_else(|| panic!("no log path on stderr:\n{stderr}"));
    fs::canonicalize(named).unwrap_or_else(|err| panic!("{named}: {err}"))
}

#[then(expr = "the pack holds no log file")]
fn the_pack_holds_no_log_file(world: &mut World) {
    for (path, _) in host::read_files(host::pack(world)) {
        let is_log = path.extension().is_some_and(|extension| extension == "log");
        assert!(!is_log, "{}", path.display());
    }
}

#[then(expr = "the host reports the game port and the MCP address")]
fn the_host_reports_the_game_port_and_the_mcp_addr(world: &mut World) {
    let live = host::live_mut(world);
    let stdout = live
        .process
        .as_ref()
        .map(|server| support::text_of(&server.stdout))
        .unwrap_or_default();
    let game = live.game_port;
    let mcp_port = live.mcp_port;
    let (reported_game, reported_mcp_addr) = host::parse_addrs(&stdout);
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

#[when(expr = "two tables listen on port 0 at the same time")]
async fn two_tables_listen_on_port_0_at_the_same_time(world: &mut World) {
    let first = host::pack(world).to_path_buf();
    let second = copy_pack("docks", "ports-other");
    world.scratch = second.to_string_lossy().into_owned();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let flags = ["--addr", "127.0.0.1:0"];
    let started_first = host::spawn(dir, &first, &flags);
    let started_second = host::spawn(dir, &second, &flags);
    let first = host::wait_addrs(started_first).await;
    let second = host::wait_addrs(started_second).await;
    world.stdout = format!(
        "{} {} {} {}",
        first.game, first.mcp_addr, second.game, second.mcp_addr
    );
    world.plays.push(first.process);
    world.plays.push(second.process);
}

#[then(expr = "the tables report distinct ports")]
fn the_tables_report_distinct_addresses(world: &mut World) {
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
}

#[then(expr = "the tables name different log files")]
async fn the_tables_name_different_log_files(world: &mut World) {
    let mut files: Vec<PathBuf> = Vec::new();
    for host in &world.plays {
        files.push(wait_for_log_file(&host.stderr).await);
    }
    assert_ne!(files[0], files[1]);
    for file in &files {
        let text =
            fs::read_to_string(file).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
        assert_eq!(text.matches("pack opened").count(), 1, "{text}");
    }
    world.plays.clear();
    let _ = fs::remove_dir_all(&world.scratch);
}
