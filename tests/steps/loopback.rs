use std::net::TcpStream;

use cucumber::{then, when};

use crate::World;

use super::table;

#[when(expr = "the operator runs run bound to {string}")]
fn run_bound_to(world: &mut World, addr: String) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["run", pack.to_str().unwrap(), "--bind", &addr]);
}

#[when(expr = "the operator runs run with --as")]
fn run_with_as(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(
        world,
        &["run", pack.to_str().unwrap(), "--as", "player:pc_rowan"],
    );
}

#[when(expr = "the operator runs run with mcp and bind")]
fn run_with_mcp_and_bind(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(
        world,
        &[
            "run",
            pack.to_str().unwrap(),
            "--mcp",
            "pack-table",
            "--bind",
            "127.0.0.1",
        ],
    );
}

#[when(expr = "the table listens on 127.0.0.1")]
fn listens_on_default(world: &mut World) {
    table::listen(table::pack(world), "127.0.0.1", 9737);
}

#[when(expr = "the table listens on a free loopback port")]
fn listens_on_a_free_port(world: &mut World) {
    let port = table::free_port();
    let bind = format!("127.0.0.1:{port}");
    table::listen(table::pack(world), &bind, port);
}

#[then(expr = "a connection to the table succeeds")]
fn a_connection_succeeds(_world: &mut World) {
    let port = crate::with_live(|live| live.port);
    TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|err| panic!("connect: {err}"));
}

#[then(expr = "the listen port is not 9737")]
fn the_listen_port_is_not_9737(_world: &mut World) {
    let port = crate::with_live(|live| live.port);
    assert_ne!(port, 9737);
}
