use cucumber::{then, when};

use super::host;
use crate::World;

#[when(expr = "the operator runs status")]
async fn the_operator_runs_the_command(world: &mut World) {
    let pack = crate::support::pack_arg(host::pack(world));
    host::run(world, &["status", &pack]).await;
    world.scratch = format!("stdout:\n{}\nstderr:\n{}", world.stdout, world.stderr);
}

#[then(expr = "status exits 0")]
fn the_command_exits_0(world: &mut World) {
    assert_eq!(world.code, Some(0), "{}", world.scratch);
}
