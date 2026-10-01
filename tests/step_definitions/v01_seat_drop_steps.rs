use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use spookery::seat::SocketEnd;

use crate::support::{self, pack_arg, spawn, wait_exit, wait_has, wait_listen, write_stdin, Proc};
use crate::World;

// Live `spook run` / `spook play` processes cannot live on World. The port lock sits with them.
struct Live {
    lock: Option<support::PortLock>,
    server: Option<Proc>,
    client: Option<Proc>,
}

static LIVE: Mutex<Live> = Mutex::new(Live {
    lock: None,
    server: None,
    client: None,
});

fn live() -> std::sync::MutexGuard<'static, Live> {
    LIVE.lock().unwrap_or_else(|err| err.into_inner())
}

fn pid_file() -> PathBuf {
    std::env::temp_dir().join("spook-bdd-v01-seat-drop.pids")
}

fn remember_pids() {
    let held = live();
    let mut text = String::new();
    if let Some(server) = held.server.as_ref() {
        text.push_str(&server.child.id().to_string());
        text.push('\n');
    }
    if let Some(client) = held.client.as_ref() {
        text.push_str(&client.child.id().to_string());
        text.push('\n');
    }
    drop(held);
    let _ = fs::write(pid_file(), text);
}

fn kill_pid(pid: u32) {
    let listed = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output();
    let Ok(listed) = listed else {
        return;
    };
    let text = String::from_utf8_lossy(&listed.stdout);
    if !text.to_ascii_lowercase().contains("spook") {
        return;
    }
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output();
}

fn kill_remembered() {
    let Ok(text) = fs::read_to_string(pid_file()) else {
        return;
    };
    for line in text.lines() {
        if let Ok(pid) = line.parse::<u32>() {
            kill_pid(pid);
        }
    }
    let _ = fs::remove_file(pid_file());
}

fn clear_live() {
    let mut held = live();
    held.client.take();
    held.server.take();
    held.lock.take();
    drop(held);
    let _ = fs::remove_file(pid_file());
}

fn reclaim() {
    let occupied = {
        let held = live();
        held.server.is_some() || held.client.is_some() || held.lock.is_some()
    };
    if occupied {
        clear_live();
    } else {
        kill_remembered();
    }
}

fn pack_of(w: &World) -> &Path {
    w.pack.as_ref().unwrap()
}

fn now_of(pack: &Path) -> spookery_kernel::model::Now {
    spookery_kernel::open_shard(pack)
        .unwrap_or_else(|err| panic!("open: {err}"))
        .now()
        .clone()
}

fn wait_humans(pack: &Path, humans: u32, timeout: Duration) -> spookery_kernel::model::Now {
    let start = Instant::now();
    loop {
        if let Ok(shard) = spookery_kernel::open_shard(pack) {
            if shard.now().presence.humans == humans {
                return shard.now().clone();
            }
        }
        if start.elapsed() > timeout {
            panic!("humans did not become {humans}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn release(w: &mut World, end: SocketEnd) {
    let mut shard = spookery_kernel::open_shard(pack_of(w))
        .unwrap_or_else(|err| panic!("open: {err}"));
    let session = w.scratch.get("session_id").unwrap().clone();
    assert!(spookery::seat::release_player_seat(&mut shard, &session, "pc_rowan", end).unwrap());
}

#[given(expr = "the player seat port is locked and the shard is listening")]
fn listen(w: &mut World) {
    reclaim();
    let mut held = live();
    held.lock = Some(support::lock_port());
    drop(held);
    let mut server = spawn(&["run", &pack_arg(pack_of(w))]);
    wait_listen(&mut server);
    {
        let mut held = live();
        held.server = Some(server);
    }
    remember_pids();
}

#[when(expr = "the player client connects and perceives")]
fn connect(w: &mut World) {
    let client = spawn(&["play", &pack_arg(pack_of(w))]);
    wait_has(
        &client.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    {
        let mut held = live();
        held.client = Some(client);
    }
    remember_pids();
}

#[when(expr = "the player stdin closes")]
fn stdin_closes(_w: &mut World) {
    let mut held = live();
    held.client.as_mut().unwrap().stdin.take();
}

#[when(expr = "the player sends close")]
fn sends_close(_w: &mut World) {
    let mut held = live();
    write_stdin(held.client.as_mut().unwrap(), "close");
}

#[then(expr = "the player client exits 0")]
fn client_exits_0(_w: &mut World) {
    let mut held = live();
    let status = wait_exit(held.client.as_mut().unwrap(), Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
}

#[then(expr = "presence humans is {int}")]
fn humans_is(w: &mut World, n: u64) {
    let now = now_of(pack_of(w));
    assert_eq!(u64::from(now.presence.humans), n);
}

#[then(expr = "humans returns to 0")]
fn humans_returns(w: &mut World) {
    let now = wait_humans(pack_of(w), 0, Duration::from_secs(5));
    assert_eq!(now.presence.humans, 0);
}

#[then(expr = "session_ids is empty")]
fn session_ids_empty(w: &mut World) {
    let now = now_of(pack_of(w));
    assert!(now.presence.session_ids.is_empty());
}

#[then(expr = "actors_present is empty")]
fn actors_present_empty(w: &mut World) {
    let now = now_of(pack_of(w));
    assert!(now.presence.actors_present.is_empty());
}

#[then(expr = "one session id is seated")]
fn one_session(w: &mut World) {
    let now = now_of(pack_of(w));
    assert_eq!(now.presence.session_ids.len(), 1);
}

#[when(expr = "one second passes with no player line")]
fn one_second(w: &mut World) {
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(1));
    w.scratch.insert(
        "quiet_elapsed_ms".to_string(),
        started.elapsed().as_millis().to_string(),
    );
}

#[then(expr = "that wait stayed under 30 seconds")]
fn under_30(w: &mut World) {
    let ms: u128 = w
        .scratch
        .get("quiet_elapsed_ms")
        .unwrap()
        .parse()
        .unwrap();
    assert!(ms < 30_000);
}

#[then(expr = "the player client is still running")]
fn still_running(_w: &mut World) {
    {
        let mut held = live();
        assert!(held
            .client
            .as_mut()
            .unwrap()
            .child
            .try_wait()
            .unwrap()
            .is_none());
    }
    clear_live();
}

#[when(expr = "the player seat opens for pc_rowan")]
fn seat_opens(w: &mut World) {
    reclaim();
    let mut shard = spookery_kernel::open_shard(pack_of(w))
        .unwrap_or_else(|err| panic!("open: {err}"));
    let session = spookery::seat::seat_player(&mut shard, "pc_rowan").unwrap();
    w.scratch.insert("session_id".to_string(), session);
}

#[when(expr = "the player seat ends with a read error")]
fn ends_read(w: &mut World) {
    release(w, SocketEnd::Read);
}

#[when(expr = "the player seat ends with close")]
fn ends_close(w: &mut World) {
    release(w, SocketEnd::Close);
}

#[when(expr = "the player seat ends with keepalive")]
fn ends_keepalive(w: &mut World) {
    release(w, SocketEnd::Keepalive);
}

#[then(expr = "state\\/log\\/session.jsonl does not exist")]
fn no_session_jsonl(w: &mut World) {
    assert!(!pack_of(w).join("state/log/session.jsonl").exists());
    clear_live();
}
