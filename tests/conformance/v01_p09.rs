use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use spookery_kernel::expand::expand;
use spookery_kernel::model::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::Code;

struct PortLock(std::fs::File);

impl Drop for PortLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn lock_port() -> PortLock {
    let path = std::env::temp_dir().join("spookery-9737.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .unwrap_or_else(|err| panic!("lock {path:?}: {err}"));
    file.lock().unwrap_or_else(|err| panic!("lock {path:?}: {err}"));
    PortLock(file)
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())) {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest).unwrap_or_else(|err| panic!("copy {}: {err}", entry.path().display()));
        }
    }
}

fn temp_pack(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-gate-d-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&manifest.join("packs/oneshot-docks"), &pack);
    pack
}

struct Proc {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(args: &[&str]) -> Proc {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(args);
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap_or_else(|err| panic!("spawn spook: {err}"));
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    drain(stdout, Arc::clone(&stdout_buf));
    drain(stderr, Arc::clone(&stderr_buf));
    Proc {
        child,
        stdin,
        stdout: stdout_buf,
        stderr: stderr_buf,
    }
}

fn drain(reader: impl Read + Send + 'static, slot: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => slot.lock().unwrap().push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });
}

fn text_of(slot: &Arc<Mutex<String>>) -> String {
    slot.lock().unwrap().clone()
}

fn wait_count(slot: &Arc<Mutex<String>>, needle: &str, count: usize, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = text_of(slot);
        if text.matches(needle).count() >= count {
            return text;
        }
        if start.elapsed() > timeout {
            panic!("saw {} of {needle}, wanted {count}\n{text}", text.matches(needle).count());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_has(slot: &Arc<Mutex<String>>, needle: &str, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = text_of(slot);
        if text.contains(needle) {
            return text;
        }
        if start.elapsed() > timeout {
            panic!("missing {needle} after {timeout:?}\n{text}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_listen(proc: &mut Proc) {
    let start = Instant::now();
    loop {
        if TcpStream::connect("127.0.0.1:9737").is_ok() {
            return;
        }
        if let Some(status) = proc.child.try_wait().unwrap() {
            panic!(
                "spook run exited {:?}\nstderr:\n{}",
                status.code(),
                text_of(&proc.stderr)
            );
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("listener did not open\nstderr:\n{}", text_of(&proc.stderr));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn write_stdin(proc: &mut Proc, line: &str) {
    let stdin = proc.stdin.as_mut().unwrap();
    writeln!(stdin, "{line}").unwrap();
    stdin.flush().unwrap();
}

fn pack_arg(pack: &Path) -> String {
    pack.to_str().unwrap().to_string()
}

fn tick_text(pack: &Path) -> String {
    fs::read_to_string(pack.join("state/log/tick-0001.jsonl")).unwrap_or_else(|err| panic!("tick: {err}"))
}

fn tick_value(pack: &Path) -> Value {
    let text = tick_text(pack);
    serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("json {err}: {text}"))
}

#[test]
fn v01_attempt_line_refuses_before_a_roll() {
    let pack = temp_pack("shape");
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    let mut shard = spookery_kernel::open_shard(&pack).unwrap();
    assert_eq!(shard.now().presence.humans, 0);
    let live = must_err(spookery::seat::mechanical(&mut shard, "pc_rowan", "attempt grit 3", false));
    assert_eq!(live, Code::LiveOnly);
    assert!(shard.queued_intents().is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    assert!(!pack.join("state/log/tick-0001.jsonl").exists());
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);

    spookery::seat::seat_player(&mut shard, "pc_rowan").unwrap();
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();
    for line in ["attempt", "attempt grit"] {
        let err = must_err(spookery::seat::mechanical(&mut shard, "pc_rowan", line, false));
        assert_eq!(err, Code::Schema, "{line}");
        assert!(shard.queued_intents().is_empty(), "{line}");
        assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated, "{line}");
    }
    let bad = must_err(spookery::seat::mechanical(&mut shard, "pc_rowan", "attempt sneak shadow", false));
    assert_eq!(bad, Code::NotInSpec);
    assert!(shard.queued_intents().is_empty());
    assert!(!pack.join("state/log/tick-0001.jsonl").exists());
    assert!(!pack.join("state/log/session.jsonl").exists());
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);

    let attempt = intent("attempt");
    assert!(expand(shard.canon(), shard.now(), &attempt).unwrap().is_empty());
    let mut attack = attempt.clone();
    attack.move_name = "attack".into();
    assert_eq!(expand(shard.canon(), shard.now(), &attack).unwrap_err(), Code::NotInSpec);
    let mut give = attempt.clone();
    give.move_name = "give".into();
    assert_eq!(expand(shard.canon(), shard.now(), &give).unwrap_err(), Code::NotInSpec);
    let mut take = attempt;
    take.move_name = "take".into();
    assert_eq!(expand(shard.canon(), shard.now(), &take).unwrap_err(), Code::NotInSpec);
    let _ = fs::remove_dir_all(&pack);
}

fn must_err(result: Result<spookery::seat::Mechanical, Code>) -> Code {
    match result {
        Err(code) => code,
        Ok(_) => panic!("attempt was accepted"),
    }
}

fn intent(move_name: &str) -> Intent {
    Intent {
        id: "a".to_string(),
        tick_index: 1,
        actor_id: "pc_rowan".to_string(),
        actor_kind: ActorKind::Player,
        source: Source::Player,
        move_name: move_name.to_string(),
        target: Target::Empty,
        args: IntentArgs {
            check: Some("grit".to_string()),
            check_target: Some(3),
            ..IntentArgs::default()
        },
        rationale_code: String::new(),
    }
}

#[test]
fn v01_p09_no_gm_attempt_rolls_on_send_and_writes_no_now_path() {
    let _lock = lock_port();
    let pack = temp_pack("live");
    let sheet = fs::read(pack.join("state/actors/pc_rowan.yaml")).unwrap();
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut player = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&player.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));

    write_stdin(&mut player, "attempt grit 3");
    let stdout = wait_has(&player.stdout, "{\"kind\":\"check\",\"success\":true}", Duration::from_secs(5));
    assert_eq!(player_checks(&stdout), vec!["{\"kind\":\"check\",\"success\":true}".to_string()]);
    assert!(!stdout.contains("\"faces\""));
    assert!(!text_of(&player.stderr).contains("NOT_IN_SPEC"));

    let now = spookery_kernel::open_shard(&pack).unwrap().now().clone();
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.world_time, 1085);
    assert_eq!(now.tick_index, 1);
    assert_eq!(fs::read(pack.join("state/actors/pc_rowan.yaml")).unwrap(), sheet);

    let raw = tick_text(&pack);
    let tick = tick_value(&pack);
    let mut keys: Vec<_> = tick.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "effects",
            "intents",
            "last_tick_at",
            "presence_humans",
            "rejected",
            "seed",
            "state_rev_after",
            "state_rev_before",
            "tick_index",
            "tool_trace",
            "world_time_after",
            "world_time_before",
        ]
    );
    let mut cursor = 0;
    for key in [
        "tick_index",
        "state_rev_before",
        "state_rev_after",
        "world_time_before",
        "world_time_after",
        "presence_humans",
        "seed",
        "last_tick_at",
        "intents",
        "rejected",
        "effects",
        "tool_trace",
    ] {
        let found = raw[cursor..].find(&format!("\"{key}\"")).unwrap_or_else(|| panic!("missing {key} in {raw}"));
        cursor += found + 1;
    }
    assert_no_roll_keys(&tick, false);
    let attempt = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|intent| intent["move"] == "attempt")
        .unwrap();
    assert_eq!(attempt["args"]["check"], "grit");
    assert_eq!(attempt["args"]["target"], 3);
    assert!(attempt["args"]["target"].is_i64());
    assert!(tick["tool_trace"]
        .as_array()
        .unwrap()
        .iter()
        .any(|name| name == "resolve_check"));

    let twin = temp_pack("wait-twin");
    let mut waited = spookery_kernel::open_shard(&twin).unwrap();
    spookery::seat::seat_player(&mut waited, "pc_rowan").unwrap();
    spookery::seat::queue_wait(&mut waited, "pc_rowan").unwrap();
    assert!(spookery::seat::commit_queued(&mut waited).unwrap());
    assert!(!now.clocks.get("assassination_debt").unwrap().paused);
    assert_eq!(waited.now().actors, now.actors);
    assert_eq!(waited.now().pause, now.pause);
    assert_eq!(waited.now().clocks, now.clocks);
    assert_eq!(waited.now().places, now.places);
    assert_eq!(waited.now().rumors, now.rumors);
    assert_eq!(waited.now().messages, now.messages);
    assert_eq!(waited.now().proposals, now.proposals);
    assert_eq!(waited.now().world_time, now.world_time);
    assert_eq!(kernel_moves(&tick_value(&twin)), kernel_moves(&tick));

    write_stdin(&mut player, "attempt grit 14");
    let failed = wait_has(&player.stdout, "{\"kind\":\"check\",\"success\":false}", Duration::from_secs(5));
    assert_eq!(
        player_checks(&failed),
        vec![
            "{\"kind\":\"check\",\"success\":true}".to_string(),
            "{\"kind\":\"check\",\"success\":false}".to_string(),
        ]
    );
    assert_eq!(fs::read(pack.join("state/actors/pc_rowan.yaml")).unwrap(), sheet);
    let _ = fs::remove_dir_all(&pack);
    let _ = fs::remove_dir_all(&twin);
}

#[test]
fn v01_p09_gm_attempt_rolls_on_commit_and_the_gm_sees_faces() {
    let _lock = lock_port();
    let pack = temp_pack("gm");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut gm = gm_session();
    let mut player = spawn(&["play", &pack_arg(&pack)]);
    let hello = wait_has(&player.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    write_stdin(&mut player, "attempt grit 3");
    let queued = wait_has(&player.stdout, "\"kind\":\"queued\"", Duration::from_secs(5));
    assert!(!queued[hello.len()..].contains("\"kind\":\"check\""), "{queued}");
    assert!(!queued[hello.len()..].contains("\"kind\":\"resume\""), "{queued}");
    let seated = spookery_kernel::open_shard(&pack).unwrap().now().clone();
    assert_eq!(seated.world_time, 1080);
    assert!(tick_text_missing(&pack));

    let _queued_events = drain_ready(&mut gm);
    write!(
        gm,
        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"commit_turn\",\"arguments\":{{\"include_tick\":true}}}}}}\n"
    )
    .unwrap();
    gm.flush().unwrap();
    let lines = read_until(&mut gm, |lines| {
        lines.iter().any(|line| line.contains("\"id\":2")) && lines.iter().any(|line| line.contains("\"faces\""))
    });
    let lines = finish_gm(&mut gm, lines);
    let checks: Vec<_> = lines.iter().filter(|line| line.contains("\"kind\":\"check\"")).collect();
    assert_eq!(checks.len(), 1, "{lines:?}");
    let result_at = lines.iter().position(|line| line.contains("\"id\":2")).unwrap();
    let event_at = lines.iter().position(|line| line.contains("\"faces\"")).unwrap();
    assert!(result_at < event_at, "{lines:?}");
    let result = &lines[result_at];
    let event = &lines[event_at];
    assert!(!result.contains("faces"), "{result}");
    assert!(event.contains("\"params\":{\"kind\":\"check\",\"faces\":["), "{event}");
    assert_event_order(event);
    let params = event_params(event);
    assert_roll(&params, 3);

    let stdout = wait_has(&player.stdout, "{\"kind\":\"check\",\"success\":", Duration::from_secs(5));
    let word = if params["success"] == true { "true" } else { "false" };
    assert_eq!(
        player_checks(&stdout[queued.len()..]),
        vec![format!("{{\"kind\":\"check\",\"success\":{word}}}")]
    );
    assert_eq!(spookery_kernel::open_shard(&pack).unwrap().now().world_time, 1085);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_unknown_check_does_not_advance_the_die_or_send_a_check() {
    let _lock = lock_port();
    let pack = temp_pack("unknown");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut gm = gm_session();
    let mut player = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&player.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    write_stdin(&mut player, "attempt nope 5");
    wait_has(&player.stdout, "\"kind\":\"queued\"", Duration::from_secs(5));
    write_stdin(&mut player, "attempt grit 3");
    let twice = wait_count(&player.stdout, "\"kind\":\"queued\"", 2, Duration::from_secs(5));
    let before = twice;
    assert!(!before.contains("{\"kind\":\"check\""), "{before}");
    let _ = drain_ready(&mut gm);
    write!(
        gm,
        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"commit_turn\",\"arguments\":{{\"include_tick\":true}}}}}}\n"
    )
    .unwrap();
    gm.flush().unwrap();
    let lines = read_until(&mut gm, |lines| lines.iter().any(|line| line.contains("\"faces\"")));
    let lines = finish_gm(&mut gm, lines);
    let events: Vec<_> = lines.iter().filter(|line| line.contains("\"kind\":\"check\"")).cloned().collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "{lines:?}");
    let stderr = wait_has(&player.stderr, "UNKNOWN_KEY", Duration::from_secs(5));
    assert_eq!(stderr.trim(), "UNKNOWN_KEY");
    let stdout = wait_has(&player.stdout, "{\"kind\":\"check\",\"success\":", Duration::from_secs(5));
    let added = &stdout[before.len()..];
    let params = event_params(&events[0]);
    assert_event_order(&events[0]);
    assert_roll(&params, 3);
    let word = if params["success"] == true { "true" } else { "false" };
    assert_eq!(
        player_checks(added),
        vec![format!("{{\"kind\":\"check\",\"success\":{word}}}")]
    );

    let tick = tick_value(&pack);
    assert_no_roll_keys(&tick, false);
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(rejected.iter().any(|row| row["reason"] == "UNKNOWN_KEY"), "{tick}");
    assert!(tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|intent| intent["move"] == "attempt" && intent["args"]["check"] == "grit"));
    assert!(!tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|intent| intent["args"]["check"] == "nope"));
    assert_eq!(
        tick["tool_trace"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|name| *name == "resolve_check")
            .count(),
        2
    );

    let shard = spookery_kernel::open_shard(&pack).unwrap();
    let seed = tick["seed"].as_str().unwrap();
    let rng = spookery_kernel::blake2s_256(seed.as_bytes());
    let mut cursor = 0usize;
    let fresh = spookery_kernel::check::resolve_check(
        shard.canon(),
        shard.sheets(),
        &ViewerId::Player("pc_rowan".to_string()),
        "grit",
        3,
        &rng,
        &mut cursor,
    )
    .unwrap();
    let faces: Vec<u32> = params["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|face| face.as_u64().unwrap() as u32)
        .collect();
    assert_eq!(faces, fresh.faces);
    assert_eq!(cursor, fresh.faces.len());
    let _ = fs::remove_dir_all(&pack);
}

fn player_checks(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.starts_with("{\"kind\":\"check\""))
        .map(|line| line.to_string())
        .collect()
}

fn assert_roll(params: &Value, target: i64) {
    let faces = params["faces"].as_array().unwrap();
    assert_eq!(faces.len(), 2, "{params}");
    let sum = faces.iter().map(|face| face.as_i64().unwrap()).sum::<i64>();
    assert!(faces.iter().all(|face| (1..=6).contains(&face.as_i64().unwrap())), "{params}");
    assert_eq!(params["target"], target);
    assert_eq!(params["stat"], 1);
    assert_eq!(params["total"], sum + params["stat"].as_i64().unwrap());
    assert!(params["success"].is_boolean());
    assert_eq!(params["success"], params["total"].as_i64().unwrap() >= target);
}

fn assert_event_order(event: &str) {
    let mut cursor = 0;
    for key in ["kind", "faces", "stat", "total", "target", "success"] {
        let found = event[cursor..]
            .find(&format!("\"{key}\""))
            .unwrap_or_else(|| panic!("missing {key} in {event}"));
        cursor += found + 1;
    }
}

fn kernel_moves(tick: &Value) -> Vec<(String, String)> {
    tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["source"] == "kernel")
        .map(|intent| {
            (
                intent["actor_id"].as_str().unwrap().to_string(),
                intent["move"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn assert_no_roll_keys(value: &Value, in_args: bool) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if !in_args && matches!(key.as_str(), "faces" | "stat" | "total" | "success") {
                    panic!("roll field {key} in {value}");
                }
                if !in_args && key == "target" && child.is_number() {
                    panic!("roll target outside args in {value}");
                }
                assert_no_roll_keys(child, in_args || key == "args");
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_no_roll_keys(item, in_args);
            }
        }
        _ => {}
    }
}

fn tick_text_missing(pack: &Path) -> bool {
    !pack.join("state/log/tick-0001.jsonl").exists()
}

fn gm_session() -> TcpStream {
    let mut stream = TcpStream::connect("127.0.0.1:9737").unwrap();
    stream.set_nodelay(true).unwrap();
    stream
        .write_all(b"gm\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"proof\",\"version\":\"0\"}}}\n")
        .unwrap();
    let init = read_lf(&mut stream, Duration::from_secs(5));
    assert!(init.contains("\"result\""), "{init}");
    stream
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .unwrap();
    stream
}

fn event_params(line: &str) -> Value {
    let value: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}: {line}"));
    value["params"].clone()
}

fn read_lf(stream: &mut TcpStream, timeout: Duration) -> String {
    stream.set_read_timeout(Some(timeout)).unwrap();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => panic!("eof {}", String::from_utf8_lossy(&buf)),
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(err) => panic!("read {err}; {}", String::from_utf8_lossy(&buf)),
        }
    }
    String::from_utf8(buf).unwrap()
}

fn read_until(stream: &mut TcpStream, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    let mut lines = Vec::new();
    let start = Instant::now();
    while !done(&lines) {
        if start.elapsed() > Duration::from_secs(5) {
            panic!("gm lines stalled: {lines:?}");
        }
        lines.push(read_lf(stream, Duration::from_secs(5)));
    }
    lines
}

fn finish_gm(stream: &mut TcpStream, mut lines: Vec<String>) -> Vec<String> {
    lines.extend(drain_ready(stream));
    lines
}

fn drain_ready(stream: &mut TcpStream) -> Vec<String> {
    stream.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    let mut lines = Vec::new();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    lines.push(String::from_utf8(buf.clone()).unwrap());
                    buf.clear();
                } else {
                    buf.push(byte[0]);
                }
            }
            Err(_) => break,
        }
    }
    lines
}
