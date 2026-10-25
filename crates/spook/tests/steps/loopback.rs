use std::net::TcpStream;

use cucumber::{then, when};

use crate::World;

use super::table;

#[when(expr = "the operator runs {word} bound to {string}")]
fn command_bound_to(world: &mut World, command: String, addr: String) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &[&command, "--addr", &addr]);
    if world.code == Some(2) {
        table::run(world, &[&command, pack.to_str().unwrap(), "--addr", &addr]);
    }
}

#[when(expr = "the table listens on a free loopback port")]
fn listens_on_a_free_port(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
}

#[then(expr = "a connection to the table succeeds")]
fn a_connection_succeeds(world: &mut World) {
    let port = table::live_mut(world).port;
    TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|err| panic!("connect: {err}"));
}

#[then(expr = "the listen port is not 9737")]
fn the_listen_port_is_not_9737(world: &mut World) {
    let port = table::live_mut(world).port;
    assert_ne!(port, 9737);
}
