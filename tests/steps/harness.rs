use cucumber::{given, then, when};

use crate::support::spook;
use crate::World;

use super::table;

#[given(expr = "a docks table")]
fn a_docks_table(world: &mut World) {
    let pack = crate::support::temp_pack("status");
    world.before = table::files(&pack);
    world.pack = Some(pack);
}

#[when(expr = "the operator runs status")]
fn the_operator_runs_status(world: &mut World) {
    let pack = world
        .pack
        .as_ref()
        .unwrap_or_else(|| panic!("no docks table"));
    let output = spook(&["status", pack.to_str().unwrap()]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    world.scratch = format!("stdout:\n{}\nstderr:\n{}", world.stdout, world.stderr);
}

#[then(expr = "status exits 0")]
fn status_exits_0(world: &mut World) {
    assert_eq!(world.code, Some(0), "{}", world.scratch);
}

#[then(expr = "the pack bytes are unchanged")]
fn the_pack_bytes_are_unchanged(world: &mut World) {
    let pack = world
        .pack
        .as_ref()
        .unwrap_or_else(|| panic!("no docks table"));
    assert_eq!(table::files(pack), world.before, "status wrote the pack");
}
