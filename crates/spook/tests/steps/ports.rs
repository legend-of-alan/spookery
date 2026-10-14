use std::collections::BTreeSet;
use std::fs;
use std::net::{SocketAddr, TcpStream};

use cucumber::{then, when};

use crate::support::{self, temp_pack};
use crate::World;

use super::table;

#[when(expr = "the table listens on port 0")]
fn the_table_listens_on_port_0(world: &mut World) {
    table::listen(table::pack(world));
}

#[then(expr = "the host reports the game port and the agent door")]
fn the_host_reports_the_game_port_and_the_agent_door(_world: &mut World) {
    let (game, door, stdout) = crate::with_live(|live| {
        let stdout = live
            .server
            .as_ref()
            .map(|server| support::text_of(&server.stdout))
            .unwrap_or_default();
        (live.port, live.http_port, stdout)
    });
    let (reported_game, reported_door) = table::reported(&stdout);
    assert_eq!(reported_game.port(), game, "{stdout}");
    assert_eq!(reported_door.port(), door, "{stdout}");
    assert_ne!(game, door, "{stdout}");
    assert_ne!(game, 0, "{stdout}");
    assert_ne!(door, 0, "{stdout}");
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.contains("\"kind\":\"listening\""))
            .count(),
        1,
        "{stdout}"
    );
    TcpStream::connect(("127.0.0.1", game)).unwrap_or_else(|err| panic!("game: {err}"));
    TcpStream::connect(("127.0.0.1", door)).unwrap_or_else(|err| panic!("door: {err}"));
}

#[when(expr = "a player connects to the reported game port")]
fn a_player_connects_to_the_reported_game_port(world: &mut World) {
    table::seat_player(table::pack(world));
}

#[then(expr = "the player receives a perceive line")]
fn the_player_receives_a_perceive_line(_world: &mut World) {
    let text = crate::with_live(|live| support::text_of(&live.player_text));
    assert!(text.contains("\"kind\":\"perceive\""), "{text}");
}

#[when(expr = "two tables listen on port 0 at the same time")]
fn two_tables_listen_on_port_0_at_the_same_time(world: &mut World) {
    let first = table::pack(world).to_path_buf();
    let second = temp_pack("ports-other");
    world.scratch = second.to_string_lossy().into_owned();
    let started_first = table::spawn_table(&first, &["--addr", "127.0.0.1:0"], &[]);
    let started_second = table::spawn_table(&second, &["--addr", "127.0.0.1:0"], &[]);
    let first = table::take_report(started_first);
    let second = table::take_report(started_second);
    world.stdout = format!(
        "{} {} {} {}",
        first.game, first.door, second.game, second.door
    );
    world.plays.push(first.server);
    world.plays.push(second.server);
}

#[then(expr = "the tables report distinct ports")]
fn the_tables_report_distinct_ports(world: &mut World) {
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
