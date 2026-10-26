use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use cucumber::{then, when};

use crate::support;
use crate::World;

use super::table;

#[when(expr = "spook run seats a player")]
fn spook_run_seats_a_player(world: &mut World) {
    start_host(world, false, false);
    seat(world);
}

#[when(expr = "spook run starts with stdin closed")]
fn spook_run_starts_with_stdin_closed(world: &mut World) {
    start_host(world, true, false);
}

#[when(expr = "a player sits at that host")]
fn a_player_sits_at_that_host(world: &mut World) {
    seat(world);
}

#[when(expr = "spook run seats a player and shutdown will block")]
fn spook_run_seats_a_player_and_shutdown_will_block(world: &mut World) {
    start_host(world, false, true);
    seat(world);
}

#[when(expr = "spook run receives SIGINT")]
fn spook_run_receives_sigint(world: &mut World) {
    send_interrupt(world);
}

#[then(expr = "spook run is still serving")]
fn spook_run_is_still_serving(world: &mut World) {
    assert!(
        try_status(world).is_none(),
        "the host exited\n{}",
        host_log(world)
    );
    let text = support::text_of(&table::live_mut(world).player_text);
    assert!(text.contains("\"kind\":\"perceive\""), "{text}");
}

#[then(expr = "spook run exits 0 and spook verify passes")]
fn spook_run_exits_0_and_spook_verify_passes(world: &mut World) {
    let status = wait_exit(world);
    assert!(
        status.success(),
        "host exited {status:?}\n{}",
        host_log(world)
    );
    let pack = table::pack(world).to_path_buf();
    let output = support::spook(&["verify", &support::pack_arg(&pack)]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "verify failed\n{stdout}{stderr}");
    assert!(stdout.contains("verified"), "{stdout}");
}

#[then(expr = "spook run exits 0")]
fn spook_run_exits_0(world: &mut World) {
    let status = wait_exit(world);
    assert!(
        status.success(),
        "host exited {status:?}\n{}",
        host_log(world)
    );
}

#[then(expr = "shutdown is blocked and spook run is still running")]
fn shutdown_is_blocked_and_spook_run_is_still_running(world: &mut World) {
    let path = PathBuf::from(&world.scratch);
    let start = Instant::now();
    while !path.is_file() {
        if try_status(world).is_some() {
            panic!(
                "the host exited before shutdown blocked\n{}",
                host_log(world)
            );
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("shutdown did not block\n{}", host_log(world));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        try_status(world).is_none(),
        "the host exited\n{}",
        host_log(world)
    );
}

#[then(expr = "spook run has ended")]
fn spook_run_has_ended(world: &mut World) {
    let status = wait_exit(world);
    let _ = std::fs::remove_file(&world.scratch);
    assert_eq!(
        status.code(),
        Some(130),
        "second interrupt should end the process: {status:?}\n{}",
        host_log(world)
    );
}

fn start_host(world: &mut World, closed_stdin: bool, hold: bool) {
    let pack = table::pack(world).to_path_buf();
    let hold_text = if hold {
        let path = std::env::temp_dir().join(format!("spook-shutdown-hold-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let text = path.to_string_lossy().into_owned();
        world.scratch = text.clone();
        Some(text)
    } else {
        None
    };
    let held = hold_text.unwrap_or_default();
    let pair = [("SPOOK_SHUTDOWN_HOLD", held.as_str())];
    let env: &[(&str, &str)] = if hold { &pair } else { &[] };
    let pending = table::spawn_host(&pack, &["--addr", "127.0.0.1:0"], env, closed_stdin, true);
    let listening = table::take_report(pending);
    table::install(world, &pack, listening);
}

fn seat(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::seat_player(world, &pack);
}

fn send_interrupt(world: &mut World) {
    let pid = table::live_mut(world)
        .server
        .as_ref()
        .unwrap_or_else(|| panic!("no host"))
        .child
        .id();
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        let rc = unsafe { kill(pid as i32, 2) };
        assert_eq!(rc, 0, "kill: {}", std::io::Error::last_os_error());
    }
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
        }
        // Ctrl+C cannot be aimed at one process group. Ctrl+Break can.
        const CTRL_BREAK_EVENT: u32 = 1;
        let rc = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) };
        assert_ne!(rc, 0, "ctrl-break: {}", std::io::Error::last_os_error());
    }
}

fn try_status(world: &mut World) -> Option<ExitStatus> {
    table::live_mut(world)
        .server
        .as_mut()
        .unwrap_or_else(|| panic!("no host"))
        .child
        .try_wait()
        .unwrap_or_else(|err| panic!("wait: {err}"))
}

fn wait_exit(world: &mut World) -> ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = try_status(world) {
            return status;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("the host is still running\n{}", host_log(world));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn host_log(world: &mut World) -> String {
    let server = table::live_mut(world)
        .server
        .as_ref()
        .unwrap_or_else(|| panic!("no host"));
    format!(
        "stdout:\n{}\nstderr:\n{}",
        support::text_of(&server.stdout),
        support::text_of(&server.stderr)
    )
}
