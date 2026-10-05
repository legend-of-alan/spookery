use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, TryLockError};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};

use crate::support::{self, text_of};
use crate::World;

struct Sitting {
    server: support::Proc,
    player: Option<support::Proc>,
    // Drop keeps the GM process seated until stop.
    _gm: Option<support::Proc>,
}

fn sitting_slot() -> &'static Mutex<Option<Sitting>> {
    static SLOT: Mutex<Option<Sitting>> = Mutex::new(None);
    &SLOT
}

fn lock_slot() -> std::sync::MutexGuard<'static, Option<Sitting>> {
    match sitting_slot().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn take_sitting() -> Sitting {
    lock_slot().take().expect("no spook run")
}

fn put_sitting(sitting: Sitting) {
    let mut slot = lock_slot();
    if slot.is_some() {
        panic!("a spook run is already held");
    }
    *slot = Some(sitting);
}

fn drop_sitting() {
    lock_slot().take();
}

// A failed humans wait must not leave spook run bound to 9737 for the rerun.
fn install_cleanup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            match sitting_slot().try_lock() {
                Ok(mut guard) => {
                    guard.take();
                }
                Err(TryLockError::Poisoned(poisoned)) => {
                    poisoned.into_inner().take();
                }
                Err(TryLockError::WouldBlock) => {}
            }
            previous(info);
        }));
    });
}

struct Held {
    sitting: Option<Sitting>,
    restore: bool,
}

impl Held {
    fn take() -> Self {
        Self {
            sitting: Some(take_sitting()),
            restore: true,
        }
    }

    fn sitting(&mut self) -> &mut Sitting {
        self.sitting.as_mut().expect("sitting already dropped")
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let Some(sitting) = self.sitting.take() else {
            return;
        };
        if std::thread::panicking() || !self.restore {
            drop(sitting);
        } else {
            put_sitting(sitting);
        }
    }
}

fn pack_string(w: &World) -> String {
    support::pack_arg(w.pack.as_ref().unwrap())
}

fn copy_of(w: &World, label: &str) -> String {
    w.scratch
        .get(&format!("crash-{label}"))
        .unwrap_or_else(|| panic!("missing crash-{label}"))
        .clone()
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

fn write_crashed(pack: &Path) {
    let text = fs::read_to_string(pack.join("state/now.yaml")).unwrap();
    let mut now = spookery_kernel::codec::load_now(&text).unwrap();
    now.pack_rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    now.presence.humans = 1;
    now.presence.actors_present = vec!["pc_rowan".to_string()];
    fs::write(
        pack.join("state/now.yaml"),
        spookery_kernel::codec::emit_now(&now).unwrap(),
    )
    .unwrap();
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

fn wait_for_session_full(holder: &mut support::Proc) -> String {
    std::thread::sleep(Duration::from_millis(300));
    if let Some(status) = holder.child.try_wait().unwrap() {
        panic!(
            "gm holder exited {:?}\nstdout:\n{}\nstderr:\n{}",
            status.code(),
            text_of(&holder.stdout),
            text_of(&holder.stderr)
        );
    }
    let mut stream =
        TcpStream::connect("127.0.0.1:9737").unwrap_or_else(|err| panic!("connect: {err}"));
    stream.set_nodelay(true).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(b"gm\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .unwrap_or_else(|err| panic!("read refusal: {err}"));
    if !line.contains("SESSION_FULL") {
        panic!(
            "gm seat was not held: {line:?}\nstdout:\n{}\nstderr:\n{}",
            text_of(&holder.stdout),
            text_of(&holder.stderr)
        );
    }
    line
}

fn remember_exit(w: &mut World, label: &str, proc: &mut support::Proc, timeout: Duration) {
    let status = support::wait_exit(proc, timeout);
    w.scratch
        .insert(format!("{label}-ok"), status.success().to_string());
    w.scratch
        .insert(format!("{label}-stderr"), text_of(&proc.stderr));
}

#[given(expr = "the pack now.yaml still names session_ids")]
fn plant_session_ids(w: &mut World) {
    let path = w.pack.as_ref().unwrap().join("state/now.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let from = "presence:\n  humans: 0\n  actors_present: []\n";
    let to = "presence:\n  humans: 1\n  session_ids: [\"crash-session\"]\n  actors_present: [\"pc_rowan\"]\n";
    assert!(text.contains(from), "{text}");
    let next = text.replacen(from, to, 1);
    fs::write(&path, &next).unwrap();
    w.scratch.insert("schema-now".to_string(), next);
}

#[when(expr = "tick, status, migrate, and mcp run against that now.yaml")]
fn schema_commands(w: &mut World) {
    let pack = pack_string(w);
    let commands = [
        ("tick", vec!["tick".to_string(), pack.clone()]),
        ("status", vec!["status".to_string(), pack.clone()]),
        ("migrate", vec!["migrate".to_string(), pack.clone()]),
        (
            "mcp",
            vec![
                "run".to_string(),
                pack,
                "--mcp".to_string(),
                "pack-table".to_string(),
            ],
        ),
    ];
    for (label, args) in commands {
        let owned: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut proc = support::spawn(&owned);
        remember_exit(w, label, &mut proc, Duration::from_secs(10));
    }
}

#[then(expr = "each of those commands fails with SCHEMA")]
fn schema_failed(w: &mut World) {
    for label in ["tick", "status", "migrate", "mcp"] {
        let ok = w.scratch.get(&format!("{label}-ok")).map(String::as_str);
        let stderr = w
            .scratch
            .get(&format!("{label}-stderr"))
            .map(String::as_str)
            .unwrap_or("");
        assert_eq!(ok, Some("false"), "{label} {stderr}");
        assert!(stderr.contains("SCHEMA"), "{label} {stderr}");
    }
}

#[then(expr = "that now.yaml still names session_ids")]
fn schema_bytes_stay(w: &mut World) {
    let text = fs::read_to_string(w.pack.as_ref().unwrap().join("state/now.yaml")).unwrap();
    assert_eq!(text, *w.scratch.get("schema-now").expect("planted now"));
    assert!(text.contains("session_ids"));
}

#[given(expr = "the pack now.yaml is a crashed presence")]
fn crashed_now(w: &mut World) {
    install_cleanup();
    drop_sitting();
    write_crashed(w.pack.as_ref().unwrap());
}

#[given(expr = "crashed presence copies for tick, status, mcp, and migrate")]
fn crashed_copies(w: &mut World) {
    install_cleanup();
    drop_sitting();
    let pack = w.pack.as_ref().unwrap().clone();
    write_crashed(&pack);
    for label in ["tick", "status", "mcp", "migrate"] {
        let copy = support::temp_pack(&format!("crash-{label}"));
        support::copy_dir(&pack, &copy);
        w.scratch.insert(
            format!("crash-{label}"),
            copy.to_str()
                .unwrap_or_else(|| panic!("copy path is not utf-8"))
                .to_string(),
        );
    }
}

#[when(expr = "spook run is listening")]
fn run_listening(w: &mut World) {
    install_cleanup();
    drop_sitting();
    let pack = pack_string(w);
    let mut server = support::spawn(&["run", &pack]);
    support::wait_listen(&mut server);
    put_sitting(Sitting {
        server,
        player: None,
        _gm: None,
    });
}

#[when(expr = "stdin sends wait")]
fn send_wait(_w: &mut World) {
    let mut held = Held::take();
    support::write_stdin(&mut held.sitting().server, "wait");
}

#[when(expr = "stdin sends stop")]
fn send_stop(_w: &mut World) {
    let mut held = Held::take();
    support::write_stdin(&mut held.sitting().server, "stop");
}

#[when(expr = "the run stdin reaches EOF")]
fn run_eof(_w: &mut World) {
    let mut held = Held::take();
    held.sitting().server.stdin.take();
}

#[when(expr = "spook play is seated")]
fn play_seated(w: &mut World) {
    let pack = pack_string(w);
    let player = support::spawn(&["play", &pack]);
    support::wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let mut held = Held::take();
    held.sitting().player = Some(player);
}

#[when(expr = "the player stdin reaches EOF")]
fn player_eof(_w: &mut World) {
    let mut held = Held::take();
    held.sitting().player.as_mut().unwrap().stdin.take();
}

#[when(expr = "a GM seat is held on spook run")]
fn gm_held(w: &mut World) {
    install_cleanup();
    drop_sitting();
    let pack = pack_string(w);
    let mut before_server = support::spawn(&["run", &pack]);
    support::wait_listen(&mut before_server);
    let before = fs::read(w.pack.as_ref().unwrap().join("state/now.yaml")).unwrap();
    drop(before_server);
    w.scratch.insert("now-before".to_string(), hex(&before));
    let mut server = support::spawn(&["run", &pack]);
    support::wait_listen(&mut server);
    let mut gm = support::spawn(&["gm", &pack, "--stdio"]);
    let line = wait_for_session_full(&mut gm);
    w.scratch.insert("gm-refusal".to_string(), line);
    put_sitting(Sitting {
        server,
        player: None,
        _gm: Some(gm),
    });
}

#[when(expr = "the tick copy is ticked")]
fn tick_copy(w: &mut World) {
    let path = copy_of(w, "tick");
    let mut proc = support::spawn(&["tick", &path]);
    remember_exit(w, "tick", &mut proc, Duration::from_secs(20));
}

#[when(expr = "status runs on the status copy")]
fn status_copy(w: &mut World) {
    let path = copy_of(w, "status");
    let mut proc = support::spawn(&["status", &path]);
    remember_exit(w, "status", &mut proc, Duration::from_secs(10));
}

#[when(expr = "mcp pack-table runs on the mcp copy")]
fn mcp_copy(w: &mut World) {
    install_cleanup();
    drop_sitting();
    let path = copy_of(w, "mcp");
    let proc = support::spawn(&["run", &path, "--mcp", "pack-table"]);
    std::thread::sleep(Duration::from_millis(400));
    put_sitting(Sitting {
        server: proc,
        player: None,
        _gm: None,
    });
}

#[when(expr = "migrate runs on the migrate copy")]
fn migrate_copy(w: &mut World) {
    drop_sitting();
    let path = copy_of(w, "migrate");
    let mut proc = support::spawn(&["migrate", &path]);
    remember_exit(w, "migrate", &mut proc, Duration::from_secs(10));
}

#[when(expr = "the scheduler wakes the pack")]
fn wake_pack(w: &mut World) {
    drop_sitting();
    let pack = w.pack.as_ref().unwrap().clone();
    let mut shard = spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    spookery::schedule::wake_shard(&mut shard).unwrap_or_else(|err| panic!("wake: {err}"));
}

#[then(expr = "the run stderr is NOT_IN_SPEC")]
fn run_stderr(_w: &mut World) {
    let mut held = Held::take();
    let stderr = support::wait_has(
        &held.sitting().server.stderr,
        "NOT_IN_SPEC",
        Duration::from_secs(5),
    );
    assert_eq!(stderr.trim(), "NOT_IN_SPEC");
}

#[then(expr = "the run stdout is empty")]
fn run_stdout(_w: &mut World) {
    let mut held = Held::take();
    assert!(
        text_of(&held.sitting().server.stdout).is_empty(),
        "{}",
        text_of(&held.sitting().server.stdout)
    );
}

#[then(expr = "the run process is still running")]
fn run_alive(_w: &mut World) {
    let mut held = Held::take();
    let status = held.sitting().server.child.try_wait().unwrap();
    assert!(
        status.is_none(),
        "wait exited the process {:?}\nstdout:\n{}\nstderr:\n{}",
        status.as_ref().and_then(|status| status.code()),
        text_of(&held.sitting().server.stdout),
        text_of(&held.sitting().server.stderr)
    );
}

#[then(expr = "the run exits 0")]
fn run_exits_0(_w: &mut World) {
    let mut held = Held::take();
    let status = support::wait_exit(&mut held.sitting().server, Duration::from_secs(5));
    let stdout = text_of(&held.sitting().server.stdout);
    let stderr = text_of(&held.sitting().server.stderr);
    held.restore = false;
    assert_eq!(
        status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[then(expr = "the player exits 0")]
fn player_exits_0(_w: &mut World) {
    let mut held = Held::take();
    let status = support::wait_exit(
        held.sitting().player.as_mut().unwrap(),
        Duration::from_secs(5),
    );
    assert_eq!(status.code(), Some(0));
}

#[then(expr = "humans is {int}")]
fn humans_is(w: &mut World, humans: u64) {
    let now = now_of(w.pack.as_ref().unwrap());
    assert_eq!(now.presence.humans, u32::try_from(humans).unwrap());
}

#[then(expr = "humans returns to {int}")]
fn humans_returns(w: &mut World, humans: u64) {
    let _ = wait_humans(
        w.pack.as_ref().unwrap(),
        u32::try_from(humans).unwrap(),
        Duration::from_secs(5),
    );
}

#[then(expr = "session_ids are empty")]
fn session_ids_empty(w: &mut World) {
    let text = fs::read_to_string(w.pack.as_ref().unwrap().join("state/now.yaml")).unwrap();
    assert!(!text.contains("session_ids"), "{text}");
}

#[then(expr = "actors_present are empty")]
fn actors_empty(w: &mut World) {
    let now = now_of(w.pack.as_ref().unwrap());
    assert!(now.presence.actors_present.is_empty());
}

#[then(expr = "the second GM hello is SESSION_FULL")]
fn second_gm(w: &mut World) {
    let line = w.scratch.get("gm-refusal").expect("gm refusal");
    assert!(line.contains("SESSION_FULL"), "{line}");
}

#[then(expr = "now.yaml bytes equal the pre-GM snapshot")]
fn now_matches_snapshot(w: &mut World) {
    let bytes = fs::read(w.pack.as_ref().unwrap().join("state/now.yaml")).unwrap();
    let saved = w.scratch.get("now-before").expect("pre-GM snapshot");
    assert_eq!(&hex(&bytes), saved);
}

#[then(expr = "the tick copy opens with crashed presence")]
fn tick_opens(w: &mut World) {
    let now = now_of(Path::new(&copy_of(w, "tick")));
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.presence.actors_present, vec!["pc_rowan".to_string()]);
    let text = fs::read_to_string(Path::new(&copy_of(w, "tick")).join("state/now.yaml")).unwrap();
    assert!(!text.contains("session_ids"), "{text}");
}

#[then(expr = "the {word} copy command succeeds")]
fn copy_succeeds(w: &mut World, label: String) {
    let ok = w.scratch.get(&format!("{label}-ok")).map(String::as_str);
    let stderr = w
        .scratch
        .get(&format!("{label}-stderr"))
        .map(String::as_str)
        .unwrap_or("");
    assert_eq!(ok, Some("true"), "{stderr}");
}

#[then(expr = "the {word} copy humans is {int}")]
fn copy_humans(w: &mut World, label: String, humans: u64) {
    let now = now_of(Path::new(&copy_of(w, &label)));
    assert_eq!(now.presence.humans, u32::try_from(humans).unwrap());
}

#[then(expr = "the {word} copy tick_index is {int}")]
fn copy_tick_index(w: &mut World, label: String, tick_index: u64) {
    let now = now_of(Path::new(&copy_of(w, &label)));
    assert_eq!(now.tick_index, tick_index);
}

#[then(expr = "the {word} copy world_time is {int}")]
fn copy_world_time(w: &mut World, label: String, world_time: u64) {
    let now = now_of(Path::new(&copy_of(w, &label)));
    assert_eq!(now.world_time, world_time);
}

#[then(expr = "the {word} copy state_rev is {int}")]
fn copy_state_rev(w: &mut World, label: String, state_rev: u64) {
    let now = now_of(Path::new(&copy_of(w, &label)));
    assert_eq!(now.state_rev, state_rev);
}

#[then(expr = "the tick copy tick log is tick-0001.jsonl")]
fn tick_log(w: &mut World) {
    let pack = PathBuf::from(copy_of(w, "tick"));
    assert_eq!(tick_files(&pack), vec!["tick-0001.jsonl".to_string()]);
}

#[then(expr = "the mcp copy is still running")]
fn mcp_alive(_w: &mut World) {
    let mut held = Held::take();
    let status = held.sitting().server.child.try_wait().unwrap();
    assert!(
        status.is_none(),
        "{}",
        text_of(&held.sitting().server.stderr)
    );
}
