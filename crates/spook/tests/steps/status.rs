use cucumber::{then, when};

use crate::support::run_spook;
use crate::World;

#[when(expr = "the operator runs status")]
fn the_operator_runs_status(world: &mut World) {
    let pack = world
        .pack
        .as_ref()
        .unwrap_or_else(|| panic!("no docks table"));
    let output = run_spook(&["status", pack.to_str().unwrap()]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    world.scratch = format!("stdout:\n{}\nstderr:\n{}", world.stdout, world.stderr);
}

#[then(expr = "status exits 0")]
fn status_exits_0(world: &mut World) {
    assert_eq!(world.code, Some(0), "{}", world.scratch);
}
