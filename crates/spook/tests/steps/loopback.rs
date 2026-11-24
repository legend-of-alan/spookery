use cucumber::when;

use crate::World;

use super::host;

#[when(expr = "the operator runs {word} bound to {string}")]
async fn the_operator_runs_a_command_bound_to(world: &mut World, command: String, addr: String) {
    let pack = host::pack(world).to_path_buf();
    host::run(world, &[&command, "--addr", &addr]).await;
    if world.code == Some(2) {
        host::run(world, &[&command, pack.to_str().unwrap(), "--addr", &addr]).await;
    }
}
