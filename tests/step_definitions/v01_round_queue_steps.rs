use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cucumber::{then, when};
use rmcp::model::CallToolRequestParam;
use rmcp::ServiceExt;
use spookery_kernel::store::Shard;
use tokio::io::AsyncWriteExt;

use crate::support::{self, pack_arg, spawn, text_of, wait_count, wait_has, wait_listen, write_stdin, Proc};
use crate::World;

// Live processes, the GM client, and the in-memory shard queue cannot live on World.
struct Live {
    lock: Option<support::PortLock>,
    server: Option<Proc>,
    client: Option<Proc>,
    gm: Option<GmSeat>,
}

static LIVE: Mutex<Live> = Mutex::new(Live {
    lock: None,
    server: None,
    client: None,
    gm: None,
});

static SHARD: Mutex<Option<Shard>> = Mutex::new(None);

enum GmCmd {
    Commit {
        include_tick: bool,
        reply: std::sync::mpsc::Sender<serde_json::Value>,
    },
    Stop,
}

struct GmSeat {
    tx: tokio::sync::mpsc::Sender<GmCmd>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for GmSeat {
    fn drop(&mut self) {
        let _ = self.tx.try_send(GmCmd::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn live() -> std::sync::MutexGuard<'static, Live> {
    LIVE.lock().unwrap_or_else(|err| err.into_inner())
}

fn shard_slot() -> std::sync::MutexGuard<'static, Option<Shard>> {
    SHARD.lock().unwrap_or_else(|err| err.into_inner())
}

fn pid_file() -> PathBuf {
    std::env::temp_dir().join("spook-bdd-v01-round-queue.pids")
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
    held.gm.take();
    held.server.take();
    held.lock.take();
    drop(held);
    let _ = fs::remove_file(pid_file());
}

fn reclaim() {
    let occupied = {
        let held = live();
        held.server.is_some() || held.client.is_some() || held.gm.is_some() || held.lock.is_some()
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

fn tick_files(pack: &Path) -> Vec<String> {
    let log = pack.join("state/log");
    if !log.is_dir() {
        return Vec::new();
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&log).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        if name.starts_with("tick-") && name.ends_with(".jsonl") {
            names.push(name.into_owned());
        }
    }
    names.sort();
    names
}

fn tick_value(pack: &Path) -> serde_json::Value {
    let text = fs::read_to_string(pack.join("state/log/tick-0001.jsonl")).unwrap();
    serde_json::from_str(text.trim()).unwrap()
}

fn player_waits(tick: &serde_json::Value) -> Vec<String> {
    tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .map(|intent| intent["id"].as_str().unwrap().to_string())
        .collect()
}

fn queued_id(stdout: &str) -> String {
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("kind").and_then(|kind| kind.as_str()) == Some("queued") {
            return value
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap()
                .to_string();
        }
    }
    panic!("no queued line in {stdout}");
}

fn record_seat(w: &mut World) {
    let seated = now_of(pack_of(w));
    w.scratch
        .insert("seated_state_rev".to_string(), seated.state_rev.to_string());
    w.scratch
        .insert("seated_last_tick_at".to_string(), seated.last_tick_at);
}

fn seated_rev(w: &World) -> u64 {
    w.scratch
        .get("seated_state_rev")
        .unwrap()
        .parse()
        .unwrap()
}

async fn call_commit(
    running: &rmcp::Peer<rmcp::RoleClient>,
    include_tick: bool,
) -> serde_json::Value {
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        running.call_tool(CallToolRequestParam {
            name: "commit_turn".into(),
            arguments: Some(
                serde_json::json!({ "include_tick": include_tick })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("commit_turn timed out"))
    .unwrap_or_else(|err| panic!("commit_turn: {err}"));
    result.structured_content.unwrap()
}

fn start_gm() -> GmSeat {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap_or_else(|err| panic!("runtime: {err}"));
        rt.block_on(async move {
            let connected: Result<_, String> = async {
                let mut stream = tokio::net::TcpStream::connect("127.0.0.1:9737")
                    .await
                    .map_err(|err| err.to_string())?;
                stream
                    .write_all(b"gm\n")
                    .await
                    .map_err(|err| err.to_string())?;
                ().serve(stream).await.map_err(|err| format!("{err:?}"))
            }
            .await;
            let running = match connected {
                Ok(running) => running,
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    GmCmd::Commit { include_tick, reply } => {
                        let value = call_commit(&running, include_tick).await;
                        let _ = reply.send(value);
                    }
                    GmCmd::Stop => break,
                }
            }
        });
    });
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => GmSeat {
            tx,
            thread: Some(thread),
        },
        Ok(Err(err)) => panic!("gm seat: {err}"),
        Err(err) => panic!("gm seat ready: {err}"),
    }
}

fn gm_commit(include_tick: bool) -> serde_json::Value {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    let tx = live().gm.as_ref().unwrap().tx.clone();
    tx.try_send(GmCmd::Commit {
        include_tick,
        reply: reply_tx,
    })
    .unwrap_or_else(|err| panic!("gm send: {err}"));
    reply_rx
        .recv_timeout(Duration::from_secs(15))
        .unwrap_or_else(|err| panic!("gm reply: {err}"))
}

fn listen(w: &mut World) {
    reclaim();
    {
        let mut held = live();
        held.lock = Some(support::lock_port());
    }
    let mut server = spawn(&["run", &pack_arg(pack_of(w))]);
    wait_listen(&mut server);
    {
        let mut held = live();
        held.server = Some(server);
    }
    remember_pids();
}

fn wake(w: &mut World, player_held: bool, gm_held: bool) {
    let mut slot = shard_slot();
    let shard = slot.as_mut().unwrap();
    let advanced = spookery::seat::on_scheduled_wake(shard, player_held, gm_held)
        .unwrap_or_else(|err| panic!("wake: {err:?}"));
    w.scratch
        .insert("wake_advanced".to_string(), advanced.to_string());
}

#[when(expr = "the round queue shard is listening with no GM")]
fn listen_no_gm(w: &mut World) {
    listen(w);
}

#[when(expr = "the round queue player connects and perceives")]
fn connect(w: &mut World) {
    let client = spawn(&["play", &pack_arg(pack_of(w))]);
    let hello = wait_has(
        &client.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    w.scratch
        .insert("hello_len".to_string(), hello.len().to_string());
    w.scratch.insert(
        "hello_resume_count".to_string(),
        hello.matches("\"kind\":\"resume\"").count().to_string(),
    );
    {
        let mut held = live();
        held.client = Some(client);
    }
    remember_pids();
    record_seat(w);
}

#[when(expr = "the round queue player sends wait")]
fn sends_wait(w: &mut World) {
    let hello_len: usize = w.scratch.get("hello_len").unwrap().parse().unwrap();
    let resumes: usize = w
        .scratch
        .get("hello_resume_count")
        .unwrap()
        .parse()
        .unwrap();
    let (stdout, stderr) = {
        let mut held = live();
        let player = held.client.as_mut().unwrap();
        write_stdin(player, "wait");
        (Arc::clone(&player.stdout), Arc::clone(&player.stderr))
    };
    let text = wait_count(
        &stdout,
        "\"kind\":\"resume\"",
        resumes + 1,
        Duration::from_secs(5),
    );
    w.scratch
        .insert("added_stdout".to_string(), text[hello_len..].to_string());
    w.scratch
        .insert("player_stderr".to_string(), text_of(&stderr));
}

#[then(expr = "the player stdout gained a resume and not a queued line")]
fn resume_not_queued(w: &mut World) {
    let added = w.scratch.get("added_stdout").unwrap();
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
    assert!(!added.contains("\"kind\":\"queued\""), "{added}");
}

#[then(expr = "the player stderr does not contain NOT_IN_SPEC")]
fn stderr_ok(w: &mut World) {
    let stderr = w.scratch.get("player_stderr").unwrap();
    assert!(!stderr.contains("NOT_IN_SPEC"), "{stderr}");
}

#[then(expr = "world_time is not {int}")]
fn world_time_is_not(w: &mut World, n: u64) {
    assert_ne!(now_of(pack_of(w)).world_time, n);
}

#[then(expr = "state_rev increased by 1 from the seated snapshot")]
fn rev_increased(w: &mut World) {
    assert_eq!(now_of(pack_of(w)).state_rev, seated_rev(w) + 1);
}

#[then(expr = "last_tick_at changed from the seated snapshot")]
fn last_tick_changed(w: &mut World) {
    let before = w.scratch.get("seated_last_tick_at").unwrap();
    assert_ne!(&now_of(pack_of(w)).last_tick_at, before);
}

#[then(expr = "the only tick file is tick-0001.jsonl")]
fn only_tick(w: &mut World) {
    assert_eq!(tick_files(pack_of(w)), vec!["tick-0001.jsonl".to_string()]);
}

#[then(expr = "tick-0001.jsonl presence_humans is {int}")]
fn tick_humans(w: &mut World, n: u64) {
    let tick = tick_value(pack_of(w));
    assert_eq!(tick["presence_humans"].as_u64(), Some(n));
}

#[then(expr = "tick-0001.jsonl world_time_before is {int}")]
fn tick_before(w: &mut World, n: u64) {
    let tick = tick_value(pack_of(w));
    assert_eq!(tick["world_time_before"].as_u64(), Some(n));
}

#[then(expr = "tick-0001.jsonl world_time_after is {int}")]
fn tick_after(w: &mut World, n: u64) {
    let tick = tick_value(pack_of(w));
    assert_eq!(tick["world_time_after"].as_u64(), Some(n));
    clear_live();
}

#[when(expr = "the round queue shard seats pc_rowan")]
fn seat(w: &mut World) {
    let mut shard = spookery_kernel::open_shard(pack_of(w))
        .unwrap_or_else(|err| panic!("open: {err}"));
    let session = spookery::seat::seat_player(&mut shard, "pc_rowan")
        .unwrap_or_else(|err| panic!("seat: {err:?}"));
    w.scratch.insert("session_id".to_string(), session);
    *shard_slot() = Some(shard);
}

#[when(expr = "a scheduled wake runs with the player held and the GM free")]
fn wake_player(w: &mut World) {
    wake(w, true, false);
}

#[when(expr = "a scheduled wake runs with the player held and the GM held")]
fn wake_both(w: &mut World) {
    wake(w, true, true);
}

#[when(expr = "a scheduled wake runs with the player free and the GM held")]
fn wake_gm(w: &mut World) {
    wake(w, false, true);
}

#[then(expr = "that wake did not advance world_time")]
fn wake_stayed(w: &mut World) {
    assert_eq!(w.scratch.get("wake_advanced").unwrap(), "false");
}

#[when(expr = "the round queue shard queues a wait for pc_rowan")]
fn queue_wait(w: &mut World) {
    let mut slot = shard_slot();
    let shard = slot.as_mut().unwrap();
    let note = spookery::seat::queue_wait(shard, "pc_rowan")
        .unwrap_or_else(|err| panic!("queue: {err:?}"));
    w.scratch.insert("queued_id".to_string(), note.id);
}

#[then(expr = "the shard queue holds 1 intent")]
fn queue_one(_w: &mut World) {
    let slot = shard_slot();
    assert_eq!(slot.as_ref().unwrap().queued_intents().len(), 1);
}

#[then(expr = "the shard queue still holds that wait")]
fn queue_same(w: &mut World) {
    let slot = shard_slot();
    let shard = slot.as_ref().unwrap();
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(
        &shard.queued_intents()[0].id,
        w.scratch.get("queued_id").unwrap()
    );
}

#[when(expr = "the round queue player seat closes")]
fn seat_closes(w: &mut World) {
    let session = w.scratch.get("session_id").unwrap().clone();
    let mut slot = shard_slot();
    let shard = slot.as_mut().unwrap();
    assert!(spookery::seat::release_player_seat(
        shard,
        &session,
        "pc_rowan",
        spookery::seat::SocketEnd::Close,
    )
    .unwrap_or_else(|err| panic!("release: {err:?}")));
}

#[when(expr = "the round queue shard is listening with a GM seat")]
fn listen_gm(w: &mut World) {
    listen(w);
    let gm = start_gm();
    live().gm = Some(gm);
}

#[when(expr = "the GM commits an empty turn")]
fn empty_commit(w: &mut World) {
    let value = gm_commit(true);
    w.scratch
        .insert("empty_commit".to_string(), value.to_string());
}

#[then(expr = "the empty commit log_path is null and the diff is empty")]
fn empty_ok(w: &mut World) {
    let empty: serde_json::Value =
        serde_json::from_str(w.scratch.get("empty_commit").unwrap()).unwrap();
    assert!(empty.get("log_path").unwrap().is_null(), "{empty}");
    assert_eq!(empty["diff_summary"].as_array().unwrap().len(), 0);
}

#[then(expr = "state_rev is still the seated snapshot")]
fn rev_same(w: &mut World) {
    assert_eq!(now_of(pack_of(w)).state_rev, seated_rev(w));
}

#[when(expr = "the round queue player sends wait while a GM is seated")]
fn sends_wait_gm(w: &mut World) {
    let stdout = {
        let mut held = live();
        let player = held.client.as_mut().unwrap();
        let before = text_of(&player.stdout);
        write_stdin(player, "wait");
        let slot = Arc::clone(&player.stdout);
        drop(held);
        let stdout = wait_has(&slot, "\"kind\":\"queued\"", Duration::from_secs(5));
        (before, stdout)
    };
    let (before, stdout) = stdout;
    let added = stdout[before.len()..].to_string();
    let queued = queued_id(&stdout);
    w.scratch.insert("added_stdout".to_string(), added);
    w.scratch.insert("queued_id".to_string(), queued);
}

#[then(expr = "the player stdout gained a queued line and not a resume")]
fn queued_not_resume(w: &mut World) {
    let added = w.scratch.get("added_stdout").unwrap();
    assert!(added.contains("\"kind\":\"queued\""), "{added}");
    assert!(!added.contains("\"kind\":\"resume\""), "{added}");
}

#[when(expr = "the GM commits the queued turn")]
fn commit_queued(w: &mut World) {
    let value = gm_commit(true);
    w.scratch.insert("commit".to_string(), value.to_string());
}

#[then(expr = "the commit has no code and log_path ends with tick-0001.jsonl")]
fn commit_ok(w: &mut World) {
    let committed: serde_json::Value =
        serde_json::from_str(w.scratch.get("commit").unwrap()).unwrap();
    assert!(committed.get("code").is_none(), "{committed}");
    assert!(committed["log_path"]
        .as_str()
        .unwrap()
        .ends_with("tick-0001.jsonl"));
}

#[then(expr = "tick-0001.jsonl player waits are the queued id")]
fn waits_match(w: &mut World) {
    let tick = tick_value(pack_of(w));
    let waits = player_waits(&tick);
    assert_eq!(waits, vec![w.scratch.get("queued_id").unwrap().clone()]);
}
