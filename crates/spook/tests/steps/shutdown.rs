use cucumber::{then, when};

use crate::support::{self, Proc};
use crate::World;

use super::table;

#[when(expr = "spook run seats a player after its stdin closes")]
fn spook_run_seats_a_player_after_its_stdin_closes(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    drop(host(world).stdin.take());
    table::seat_player(world, &pack);
}

#[when(expr = "spook run receives SIGINT")]
fn spook_run_receives_sigint(world: &mut World) {
    let pid = host(world).child.id().to_string();
    let status = std::process::Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .unwrap_or_else(|err| panic!("kill: {err}"));
    assert!(status.success(), "kill -INT {pid}: {status:?}");
}

#[then(expr = "spook run exits 0 and spook verify passes")]
fn spook_run_exits_0_and_spook_verify_passes(world: &mut World) {
    let host = host(world);
    let status = host
        .child
        .wait()
        .unwrap_or_else(|err| panic!("wait: {err}"));
    let stderr = support::text_of(&host.stderr);
    assert!(status.success(), "host exited {status:?}\n{stderr}");
    let pack = table::pack(world).to_path_buf();
    let output = support::spook(&["verify", &support::pack_arg(&pack)]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "verify failed\n{stdout}");
    assert!(stdout.contains("verified"), "{stdout}");
}

fn host(world: &mut World) -> &mut Proc {
    table::live_mut(world)
        .server
        .as_mut()
        .unwrap_or_else(|| panic!("no host"))
}
