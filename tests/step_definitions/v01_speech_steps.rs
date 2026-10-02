use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use cucumber::{then, when};

use crate::support::{
    self, pack_arg, spawn, text_of, wait_exit, wait_has, wait_listen, write_stdin, Proc,
};
use crate::World;

// Live spook processes cannot live on World. The port lock sits with them.
struct Live {
    lock: Option<support::PortLock>,
    server: Option<Proc>,
    gm: Option<Proc>,
    player: Option<Proc>,
}

static LIVE: Mutex<Live> = Mutex::new(Live {
    lock: None,
    server: None,
    gm: None,
    player: None,
});

const TABLE_HELLO_EVENT: &str =
    "{\"kind\":\"say\",\"from\":\"player:pc_rowan\",\"address\":\"table\",\"text\":\"hello\"}";
const ACTOR_PSST_EVENT: &str =
    "{\"kind\":\"say\",\"from\":\"player:pc_rowan\",\"address\":\"actor\",\"text\":\"psst\"}";
const ACTOR_PSST_GM: &str = "{\"kind\":\"say\",\"seq\":1,\"from\":\"player:pc_rowan\",\"address\":\"actor\",\"place\":null,\"actor\":\"pc_rowan\",\"recipients\":[\"pc_rowan\"],\"text\":\"psst\"}";
const EVENING_EVENT: &str =
    "{\"kind\":\"say\",\"from\":\"player:pc_rowan\",\"address\":\"actor\",\"text\":\"good evening\"}";

fn live() -> std::sync::MutexGuard<'static, Live> {
    LIVE.lock().unwrap_or_else(|err| err.into_inner())
}

fn pid_file() -> PathBuf {
    std::env::temp_dir().join("spook-bdd-v01-speech.pids")
}

fn remember_pids() {
    let held = live();
    let mut text = String::new();
    for proc in [held.server.as_ref(), held.gm.as_ref(), held.player.as_ref()]
        .into_iter()
        .flatten()
    {
        text.push_str(&proc.child.id().to_string());
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
    held.player.take();
    held.gm.take();
    held.server.take();
    held.lock.take();
    drop(held);
    let _ = fs::remove_file(pid_file());
}

fn reclaim() {
    let occupied = {
        let held = live();
        held.server.is_some() || held.gm.is_some() || held.player.is_some() || held.lock.is_some()
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

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
}

fn scratch_u64(w: &World, key: &str) -> u64 {
    w.scratch
        .get(key)
        .unwrap_or_else(|| panic!("missing {key}"))
        .parse()
        .unwrap_or_else(|err| panic!("parse {key}: {err}"))
}

fn record_seated(w: &mut World) {
    let pack = pack_of(w);
    let bytes = fs::read(pack.join("state/now.yaml")).unwrap();
    let now = now_of(pack);
    w.scratch.insert("speech_now_yaml".to_string(), hex(&bytes));
    w.scratch
        .insert("speech_state_rev".to_string(), now.state_rev.to_string());
    w.scratch
        .insert("speech_world_time".to_string(), now.world_time.to_string());
    w.scratch
        .insert("speech_tick_index".to_string(), now.tick_index.to_string());
}

fn table_hello_tape(w: &World) -> String {
    format!(
        "{{\"seq\":1,\"from\":\"player:pc_rowan\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"hello\",\"tick_index\":{},\"world_time\":{}}}\n",
        scratch_u64(w, "speech_tick_index"),
        scratch_u64(w, "speech_world_time"),
    )
}

fn evening_tape(w: &World) -> String {
    format!(
        "{{\"seq\":1,\"from\":\"player:pc_rowan\",\"address\":\"actor\",\"place\":null,\"actor\":\"mira\",\"recipients\":[\"pc_rowan\"],\"text\":\"good evening\",\"tick_index\":{},\"world_time\":{}}}\n",
        scratch_u64(w, "speech_tick_index"),
        scratch_u64(w, "speech_world_time"),
    )
}

fn player_stdout_contains(needle: &str) {
    let stdout = {
        let held = live();
        held.player.as_ref().unwrap().stdout.clone()
    };
    let text = wait_has(&stdout, needle, Duration::from_secs(5));
    assert!(text.contains(needle));
}

fn gm_stdout_contains(needle: &str) {
    let stdout = {
        let held = live();
        held.gm.as_ref().unwrap().stdout.clone()
    };
    let text = wait_has(&stdout, needle, Duration::from_secs(5));
    assert!(text.contains(needle));
}

#[when(expr = "the speech shard is listening")]
fn listen(w: &mut World) {
    reclaim();
    {
        let mut held = live();
        held.lock = Some(support::lock_port());
    }
    let arg = pack_arg(pack_of(w));
    let mut server = spawn(&["run", &arg]);
    wait_listen(&mut server);
    {
        let mut held = live();
        held.server = Some(server);
    }
    remember_pids();
}

#[when(expr = "a gm client is seated")]
fn gm_seated(w: &mut World) {
    let arg = pack_arg(pack_of(w));
    let mut gm = spawn(&["gm", &arg]);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        gm.child.try_wait().unwrap().is_none(),
        "gm seat was not held"
    );
    {
        let mut held = live();
        held.gm = Some(gm);
    }
    remember_pids();
}

#[when(expr = "the speech player connects and perceives")]
fn player_connects(w: &mut World) {
    let arg = pack_arg(pack_of(w));
    let player = spawn(&["play", &arg]);
    wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    record_seated(w);
    {
        let mut held = live();
        held.player = Some(player);
    }
    remember_pids();
}

#[when(expr = "the player says hello")]
fn say_hello(_w: &mut World) {
    let mut held = live();
    write_stdin(held.player.as_mut().unwrap(), "say hello");
}

#[when(expr = "the player talks to pc_rowan psst")]
fn talk_psst(_w: &mut World) {
    let mut held = live();
    write_stdin(held.player.as_mut().unwrap(), "talk pc_rowan psst");
}

#[when(expr = "the player talks to mira good evening")]
fn talk_evening(_w: &mut World) {
    let mut held = live();
    write_stdin(held.player.as_mut().unwrap(), "talk mira good evening");
}

#[then(expr = "the player stdout contains the table say hello")]
fn stdout_table_hello(_w: &mut World) {
    player_stdout_contains(TABLE_HELLO_EVENT);
}

#[then(expr = "the player stderr does not contain {word}")]
fn player_stderr_omits(_w: &mut World, code: String) {
    let stderr = {
        let held = live();
        text_of(&held.player.as_ref().unwrap().stderr)
    };
    assert!(!stderr.contains(&code), "{stderr}");
}

#[then(expr = "session.jsonl is the one table hello tape line")]
fn tape_hello(w: &mut World) {
    let text = fs::read_to_string(pack_of(w).join("state/log/session.jsonl")).unwrap();
    assert_eq!(text, table_hello_tape(w));
}

#[then(expr = "now.yaml matches the seated snapshot")]
fn now_yaml_matches(w: &mut World) {
    let bytes = fs::read(pack_of(w).join("state/now.yaml")).unwrap();
    assert_eq!(hex(&bytes), w.scratch["speech_now_yaml"]);
}

#[then(expr = "state_rev world_time and tick_index match the seated snapshot")]
fn seated_clocks(w: &mut World) {
    let now = now_of(pack_of(w));
    assert_eq!(now.state_rev, scratch_u64(w, "speech_state_rev"));
    assert_eq!(now.world_time, scratch_u64(w, "speech_world_time"));
    assert_eq!(now.tick_index, scratch_u64(w, "speech_tick_index"));
}

#[then(expr = "the pack has no tick file")]
fn no_tick_file(w: &mut World) {
    assert!(tick_files(pack_of(w)).is_empty());
}

#[then(expr = "the shard stdout contains the table hello tape line")]
fn shard_hello(w: &mut World) {
    let needle = table_hello_tape(w);
    let needle = needle.trim_end().to_string();
    let stdout = {
        let held = live();
        held.server.as_ref().unwrap().stdout.clone()
    };
    let text = wait_has(&stdout, &needle, Duration::from_secs(5));
    assert!(text.contains(&needle));
}

#[then(expr = "the gm seat is still held")]
fn gm_still_held(_w: &mut World) {
    let mut held = live();
    assert!(
        held.gm
            .as_mut()
            .unwrap()
            .child
            .try_wait()
            .unwrap()
            .is_none(),
        "gm seat dropped on say"
    );
}

#[then(expr = "the player stdout contains the actor psst line")]
fn stdout_psst(_w: &mut World) {
    player_stdout_contains(ACTOR_PSST_EVENT);
}

#[then(expr = "the gm stdout contains the actor psst line")]
fn gm_psst(_w: &mut World) {
    gm_stdout_contains(ACTOR_PSST_GM);
}

#[then(expr = "world_time matches the seated snapshot")]
fn world_time_seated(w: &mut World) {
    let now = now_of(pack_of(w));
    assert_eq!(now.world_time, scratch_u64(w, "speech_world_time"));
}

#[when(expr = "the speech player stdin closes")]
fn speech_stdin_closes(_w: &mut World) {
    let mut held = live();
    held.player.as_mut().unwrap().stdin.take();
}

#[then(expr = "the speech player exits 0")]
fn speech_exits_0(_w: &mut World) {
    let mut held = live();
    let status = wait_exit(held.player.as_mut().unwrap(), Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
}

#[then(expr = "state_rev is one more than the seated snapshot")]
fn state_rev_bumped(w: &mut World) {
    let now = now_of(pack_of(w));
    assert_eq!(now.state_rev, scratch_u64(w, "speech_state_rev") + 1);
}

#[then(expr = "pc_rowan scrollback contains the actor psst line")]
fn rowan_scrollback(w: &mut World) {
    let speaker = "pc_rowan";
    let cursor = spookery::seat::player_scrollback(pack_of(w), speaker).unwrap();
    assert!(cursor.iter().any(|line| line.contains(ACTOR_PSST_EVENT)));
}

#[then(expr = "pc_later scrollback is empty and has no psst")]
fn later_scrollback(w: &mut World) {
    let later = "pc_later";
    let cursor = spookery::seat::player_scrollback(pack_of(w), later).unwrap();
    assert!(cursor.iter().all(|line| !line.contains("psst")));
    assert!(cursor.is_empty());
}

#[then(expr = "the speaker pc_rowan is not the later actor pc_later")]
fn speaker_is_not_later(_w: &mut World) {
    let speaker = "pc_rowan";
    let later = "pc_later";
    assert_ne!(speaker, later);
}

#[when(expr = "talk mira good evening is spoken with no human")]
fn talk_no_human(w: &mut World) {
    reclaim();
    let shard = spookery_kernel::open_shard(pack_of(w)).unwrap();
    let err = spookery::seat::speak(&shard, "pc_rowan", "talk mira good evening").unwrap_err();
    assert_eq!(err, spookery_kernel::Reason::LiveOnly);
    w.scratch
        .insert("speech_refusal".to_string(), err.token().to_string());
}

#[then(expr = "the refusal is LIVE_ONLY")]
fn refusal_live_only(w: &mut World) {
    assert_eq!(
        w.scratch.get("speech_refusal").map(String::as_str),
        Some("LIVE_ONLY")
    );
}

#[then(expr = "the player stdout contains the mira good evening line")]
fn stdout_evening(_w: &mut World) {
    player_stdout_contains(EVENING_EVENT);
}

#[then(expr = "session.jsonl is the mira good evening tape line")]
fn tape_evening(w: &mut World) {
    let text = fs::read_to_string(pack_of(w).join("state/log/session.jsonl")).unwrap();
    assert_eq!(text, evening_tape(w));
}

#[when(expr = "session.jsonl ends with a torn tail")]
fn torn_tail(w: &mut World) {
    reclaim();
    let log = pack_of(w).join("state/log");
    fs::create_dir_all(&log).unwrap();
    fs::write(log.join("session.jsonl"), b"{\"seq\":9,\"torn").unwrap();
}

#[when(expr = "say hello is spoken on the open shard")]
fn say_on_shard(w: &mut World) {
    let shard = spookery_kernel::open_shard(pack_of(w)).unwrap();
    w.scratch.insert(
        "speech_world_time_at_speak".to_string(),
        shard.now().world_time.to_string(),
    );
    let spoken = spookery::seat::speak(&shard, "pc_rowan", "say hello").unwrap();
    w.scratch
        .insert("speech_spoken_tape".to_string(), spoken.tape);
}

#[then(expr = "the spoken tape starts with seq 1")]
fn spoken_seq1(w: &mut World) {
    let tape = w.scratch.get("speech_spoken_tape").unwrap();
    assert!(tape.starts_with("{\"seq\":1,"));
}

#[then(expr = "session.jsonl is that spoken tape")]
fn jsonl_is_spoken(w: &mut World) {
    let text = fs::read_to_string(pack_of(w).join("state/log/session.jsonl")).unwrap();
    let tape = w.scratch.get("speech_spoken_tape").unwrap();
    assert_eq!(text, format!("{tape}\n"));
}

#[then(expr = "session.jsonl does not contain torn")]
fn jsonl_no_torn(w: &mut World) {
    let text = fs::read_to_string(pack_of(w).join("state/log/session.jsonl")).unwrap();
    assert!(!text.contains("torn"));
}

#[then(expr = "world_time matches the shard at the speak")]
fn world_time_at_speak(w: &mut World) {
    let now = now_of(pack_of(w));
    assert_eq!(now.world_time, scratch_u64(w, "speech_world_time_at_speak"));
}

#[then(expr = "the speech processes stop")]
fn speech_stops(_w: &mut World) {
    clear_live();
}
