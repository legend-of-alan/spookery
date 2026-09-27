use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rmcp::model::CallToolRequestParam;
use rmcp::ServiceExt;
use tokio::io::AsyncWriteExt;

fn lock_port() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())) {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest).unwrap_or_else(|err| panic!("copy {}: {err}", entry.path().display()));
        }
    }
}

fn temp_pack(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-run-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);
    pack
}

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
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

fn pack_arg(pack: &Path) -> String {
    pack.to_str().unwrap_or_else(|| panic!("pack path is not utf-8")).to_string()
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

fn wait_exit(proc: &mut Proc, timeout: Duration) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = proc.child.try_wait().unwrap() {
            return status;
        }
        if start.elapsed() > timeout {
            panic!(
                "process still running\nstdout:\n{}\nstderr:\n{}",
                text_of(&proc.stdout),
                text_of(&proc.stderr)
            );
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
                "spook run exited {:?}\nstdout:\n{}\nstderr:\n{}",
                status.code(),
                text_of(&proc.stdout),
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

fn now_of(pack: &Path) -> spookery_kernel::model::Now {
    spookery_kernel::open_shard(pack)
        .unwrap_or_else(|err| panic!("open: {err}"))
        .now()
        .clone()
}

fn wait_for_session_full(holder: &mut Proc) -> String {
    std::thread::sleep(Duration::from_millis(300));
    if let Some(status) = holder.child.try_wait().unwrap() {
        panic!(
            "gm holder exited {:?}\nstdout:\n{}\nstderr:\n{}",
            status.code(),
            text_of(&holder.stdout),
            text_of(&holder.stderr)
        );
    }
    let mut stream = TcpStream::connect("127.0.0.1:9737").unwrap_or_else(|err| panic!("connect: {err}"));
    stream.set_nodelay(true).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
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

fn refuse_line(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let n = reader.read_line(&mut line).unwrap();
    assert!(n > 0, "connection closed before the refused line");
    let mut extra = [0u8; 1];
    assert_eq!(reader.read(&mut extra).unwrap(), 0, "shard left the connection open");
    line
}

#[test]
fn stdin_is_not_a_play_door() {
    let _lock = lock_port();
    let pack = temp_pack("stdin");
    let before = files(&pack);
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    write_stdin(&mut server, "wait");
    let stderr = wait_has(&server.stderr, "NOT_IN_SPEC", Duration::from_secs(5));
    assert_eq!(stderr.trim(), "NOT_IN_SPEC");
    assert!(text_of(&server.stdout).is_empty());
    assert!(server.child.try_wait().unwrap().is_none(), "wait exited the process");
    assert_eq!(files(&pack), before, "stdin wait wrote the pack");
    assert!(tick_files(&pack).is_empty());
    write_stdin(&mut server, "stop");
    let status = wait_exit(&mut server, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    assert_eq!(files(&pack), before, "stop with no seat wrote the pack");
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn run_as_is_not_in_spec() {
    let _lock = lock_port();
    let pack = temp_pack("as");
    let before = files(&pack);
    let mut proc = spawn(&["run", &pack_arg(&pack), "--as", "player:pc_rowan"]);
    let status = wait_exit(&mut proc, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert_eq!(text_of(&proc.stderr).trim(), "NOT_IN_SPEC");
    assert!(text_of(&proc.stdout).is_empty());
    assert_eq!(files(&pack), before);
    assert_port_is_free();
    let _ = fs::remove_dir_all(&pack);
}

fn assert_port_is_free() {
    let _listener = std::net::TcpListener::bind("127.0.0.1:9737")
        .unwrap_or_else(|err| panic!("listener still open: {err}"));
}

#[test]
fn bind_other_than_loopback_is_not_in_spec() {
    let _lock = lock_port();
    let pack = temp_pack("bind");
    let before = files(&pack);
    for addr in ["0.0.0.0", "127.0.0.1:9737", "localhost"] {
        let mut proc = spawn(&["run", &pack_arg(&pack), "--bind", addr]);
        let status = wait_exit(&mut proc, Duration::from_secs(5));
        assert_eq!(status.code(), Some(1), "{addr}");
        assert_eq!(text_of(&proc.stderr).trim(), "NOT_IN_SPEC", "{addr}");
        assert!(text_of(&proc.stdout).is_empty(), "{addr}");
        assert_eq!(files(&pack), before, "{addr}");
    }
    let mut both = spawn(&[
        "run",
        &pack_arg(&pack),
        "--mcp",
        "pack-table",
        "--bind",
        "127.0.0.1",
    ]);
    let status = wait_exit(&mut both, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert_eq!(text_of(&both.stderr).trim(), "NOT_IN_SPEC");
    assert_eq!(files(&pack), before);
    assert_port_is_free();
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn bind_in_use_is_the_operating_system_error() {
    let _lock = lock_port();
    let pack = temp_pack("busy");
    let before = files(&pack);
    let _held = std::net::TcpListener::bind("127.0.0.1:9737").unwrap();
    let again = std::net::TcpListener::bind("127.0.0.1:9737").unwrap_err();
    let mut proc = spawn(&["run", &pack_arg(&pack)]);
    let status = wait_exit(&mut proc, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert!(text_of(&proc.stdout).is_empty());
    assert_eq!(text_of(&proc.stderr).trim(), again.to_string());
    assert!(!text_of(&proc.stderr).contains("NOT_IN_SPEC"));
    assert_eq!(files(&pack), before, "failed bind wrote the pack");
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn clients_without_a_listener_use_the_operating_system_error() {
    let _lock = lock_port();
    let pack = temp_pack("down");
    let before = files(&pack);
    let expected = TcpStream::connect("127.0.0.1:9737").unwrap_err();
    let only_yaml = {
        let dir = std::env::temp_dir().join(format!("spook-yaml-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::copy(pack.join("pack.yaml"), dir.join("pack.yaml")).unwrap();
        dir
    };
    for args in [
        vec!["play".to_string(), pack_arg(&pack)],
        vec!["play".to_string(), pack_arg(&only_yaml)],
        vec!["gm".to_string(), pack_arg(&pack)],
        vec!["gm".to_string(), pack_arg(&only_yaml), "--stdio".to_string()],
        vec!["play".to_string(), pack_arg(&pack), "--as".to_string(), "player:pc_rowan".to_string()],
    ] {
        let owned: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut proc = spawn(&owned);
        let status = wait_exit(&mut proc, Duration::from_secs(5));
        assert_eq!(status.code(), Some(1), "{args:?}");
        assert!(text_of(&proc.stdout).is_empty(), "{args:?}");
        assert_eq!(text_of(&proc.stderr).trim(), expected.to_string(), "{args:?}");
        assert!(!text_of(&proc.stderr).contains("NOT_IN_SPEC"), "{args:?}");
    }
    assert_eq!(files(&pack), before);
    let _ = fs::remove_dir_all(&pack);
    let _ = fs::remove_dir_all(&only_yaml);
}

#[test]
fn play_as_mira_is_schema_and_writes_nothing() {
    let _lock = lock_port();
    let pack = temp_pack("mira");
    let before = files(&pack);
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut client = spawn(&["play", &pack_arg(&pack), "--as", "player:mira"]);
    let status = wait_exit(&mut client, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert_eq!(text_of(&client.stderr).trim(), "SCHEMA");
    assert!(text_of(&client.stdout).is_empty());
    assert_eq!(files(&pack), before, "rejected play wrote the pack");
    assert_eq!(now_of(&pack).presence.humans, 0);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn player_hello_opens_one_session() {
    let _lock = lock_port();
    let pack = temp_pack("open");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let client = spawn(&["play", &pack_arg(&pack)]);
    let stdout = wait_has(&client.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    assert!(stdout.contains("{\"kind\":\"resume\""));
    assert!(client.stderr.lock().unwrap().is_empty(), "{}", text_of(&client.stderr));
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.presence.actors_present, vec!["pc_rowan".to_string()]);
    assert_eq!(now.presence.session_ids.len(), 1);
    assert_eq!(now.state_rev, 1);
    assert_eq!(now.tick_index, 0);
    assert_eq!(now.world_time, 1080);
    assert!(tick_files(&pack).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn second_player_is_session_full() {
    let _lock = lock_port();
    let pack = temp_pack("full");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let _first = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&_first.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();
    assert_eq!(now_of(&pack).presence.humans, 1);

    let mut raw = TcpStream::connect("127.0.0.1:9737").unwrap();
    raw.set_nodelay(true).unwrap();
    raw.write_all(b"player:mira\n").unwrap();
    assert_eq!(
        refuse_line(&mut raw),
        "{\"kind\":\"refused\",\"code\":\"SESSION_FULL\"}\n"
    );
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);

    let mut second = spawn(&["play", &pack_arg(&pack), "--as", "player:pc_rowan"]);
    let status = wait_exit(&mut second, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert_eq!(text_of(&second.stderr).trim(), "SESSION_FULL");
    assert!(text_of(&second.stdout).is_empty());
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);
    assert_eq!(now_of(&pack).presence.humans, 1);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn gm_connect_does_not_write_presence_and_a_second_is_full() {
    let _lock = lock_port();
    let pack = temp_pack("gm");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    let mut gm = spawn(&["gm", &pack_arg(&pack)]);
    assert_eq!(
        wait_for_session_full(&mut gm),
        "{\"kind\":\"refused\",\"code\":\"SESSION_FULL\"}\n"
    );
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    assert_eq!(now_of(&pack).presence.humans, 0);

    let mut second = spawn(&["gm", &pack_arg(&pack)]);
    let status = wait_exit(&mut second, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert_eq!(text_of(&second.stderr).trim(), "SESSION_FULL");
    assert!(text_of(&second.stdout).is_empty());
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn disconnect_and_close_drop_the_player_seat_without_a_tick() {
    let _lock = lock_port();
    let pack = temp_pack("drop");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&client.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    assert_eq!(now_of(&pack).presence.humans, 1);
    client.stdin.take();
    let status = wait_exit(&mut client, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    let dropped = wait_humans(&pack, 0, Duration::from_secs(5));
    assert_eq!(dropped.state_rev, 2);
    assert_eq!(dropped.tick_index, 0);
    assert_eq!(dropped.world_time, 1080);
    assert!(dropped.presence.session_ids.is_empty());
    assert!(tick_files(&pack).is_empty());

    let mut again = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&again.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    write_stdin(&mut again, "close");
    let status = wait_exit(&mut again, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    let closed = wait_humans(&pack, 0, Duration::from_secs(5));
    assert_eq!(closed.state_rev, 4);
    assert!(tick_files(&pack).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
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

#[test]
fn stop_and_eof_drop_a_held_seat_and_do_not_tick() {
    let _lock = lock_port();
    let pack = temp_pack("stop");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&client.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    assert_eq!(now_of(&pack).state_rev, 1);
    write_stdin(&mut server, "stop");
    let status = wait_exit(&mut server, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 0);
    assert!(now.presence.session_ids.is_empty());
    assert_eq!(now.state_rev, 2);
    assert_eq!(now.tick_index, 0);
    assert_eq!(now.world_time, 1080);
    assert!(tick_files(&pack).is_empty());

    let pack = temp_pack("eof");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&client.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    server.stdin.take();
    let status = wait_exit(&mut server, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 0);
    assert_eq!(now.state_rev, 2);
    assert!(tick_files(&pack).is_empty());

    let pack = temp_pack("gm-stop");
    let mut before_server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut before_server);
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    drop(before_server);
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut gm = spawn(&["gm", &pack_arg(&pack)]);
    assert!(wait_for_session_full(&mut gm).contains("SESSION_FULL"));
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    write_stdin(&mut server, "stop");
    let status = wait_exit(&mut server, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn bad_hellos_are_schema() {
    let _lock = lock_port();
    let pack = temp_pack("schema");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    for hello in ["\n", "foo\n", "npc:mira\n", "as=gm\n", "player:\n"] {
        let mut stream = TcpStream::connect("127.0.0.1:9737").unwrap();
        stream.set_nodelay(true).unwrap();
        stream.write_all(hello.as_bytes()).unwrap();
        assert_eq!(
            refuse_line(&mut stream),
            "{\"kind\":\"refused\",\"code\":\"SCHEMA\"}\n",
            "{hello:?}"
        );
        assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before, "{hello:?}");
    }
    assert_eq!(now_of(&pack).presence.humans, 0);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn player_lines_do_not_commit() {
    let _lock = lock_port();
    let pack = temp_pack("lines");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&client.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();
    for (index, line) in ["say hello", "talk mira", "attempt sneak shadow", "wait"].into_iter().enumerate() {
        write_stdin(&mut client, line);
        let stderr = wait_count(&client.stderr, "NOT_IN_SPEC", index + 1, Duration::from_secs(5));
        assert!(stderr.contains("NOT_IN_SPEC"), "{line}: {stderr}");
        assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated, "{line}");
    }
    assert!(client.child.try_wait().unwrap().is_none());
    assert!(tick_files(&pack).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn quiet_socket_stays_seated() {
    let _lock = lock_port();
    let pack = temp_pack("quiet");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&client.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(1));
    assert!(started.elapsed() < Duration::from_secs(30));
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.presence.session_ids.len(), 1);
    assert_eq!(now.state_rev, 1);
    assert!(tick_files(&pack).is_empty());
    assert!(client.child.try_wait().unwrap().is_none());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn simulated_read_error_drops_the_seat_like_close() {
    let pack = temp_pack("sim");
    let mut shard = spookery_kernel::open_shard(&pack).unwrap();
    let session = spookery::seat::seat_player(&mut shard, "pc_rowan").unwrap();
    assert_eq!(shard.now().presence.humans, 1);
    assert_eq!(shard.now().state_rev, 1);
    assert!(tick_files(&pack).is_empty());
    assert!(spookery::seat::release_player_seat(
        &mut shard,
        &session,
        "pc_rowan",
        spookery::seat::SocketEnd::Read
    )
    .unwrap());
    assert_eq!(shard.now().presence.humans, 0);
    assert!(shard.now().presence.session_ids.is_empty());
    assert!(shard.now().presence.actors_present.is_empty());
    assert_eq!(shard.now().state_rev, 2);
    assert_eq!(shard.now().tick_index, 0);
    assert_eq!(shard.now().world_time, 1080);
    assert!(tick_files(&pack).is_empty());

    let session = spookery::seat::seat_player(&mut shard, "pc_rowan").unwrap();
    assert!(spookery::seat::release_player_seat(
        &mut shard,
        &session,
        "pc_rowan",
        spookery::seat::SocketEnd::Close
    )
    .unwrap());
    assert_eq!(shard.now().presence.humans, 0);
    assert_eq!(shard.now().state_rev, 4);
    assert!(tick_files(&pack).is_empty());

    let session = spookery::seat::seat_player(&mut shard, "pc_rowan").unwrap();
    assert!(spookery::seat::release_player_seat(
        &mut shard,
        &session,
        "pc_rowan",
        spookery::seat::SocketEnd::Keepalive
    )
    .unwrap());
    assert_eq!(shard.now().presence.humans, 0);
    assert_eq!(shard.now().state_rev, 6);
    assert_eq!(shard.now().tick_index, 0);
    assert!(tick_files(&pack).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn crashed_presence_clears_before_accept_and_other_commands_do_not() {
    let _lock = lock_port();
    let pack = temp_pack("crash");
    write_crashed(&pack, None);
    let untouched = temp_pack("crash-tick");
    copy_dir(&pack, &untouched);
    let status_pack = temp_pack("crash-status");
    copy_dir(&pack, &status_pack);
    let mcp_pack = temp_pack("crash-mcp");
    copy_dir(&pack, &mcp_pack);
    let migrate_pack = temp_pack("crash-migrate");
    copy_dir(&pack, &migrate_pack);

    let opened = spookery_kernel::open_shard(&untouched).unwrap();
    assert_eq!(opened.now().presence.humans, 1);
    assert_eq!(opened.now().presence.session_ids, vec!["crash-session".to_string()]);
    drop(opened);

    let mut ticked = spawn(&["tick", &pack_arg(&untouched)]);
    let status = wait_exit(&mut ticked, Duration::from_secs(20));
    assert!(status.success(), "{}", text_of(&ticked.stderr));
    let now = now_of(&untouched);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.tick_index, 1);
    assert_eq!(now.world_time, 1085);
    assert_eq!(tick_files(&untouched), vec!["tick-0001.jsonl".to_string()]);

    let mut status_proc = spawn(&["status", &pack_arg(&status_pack)]);
    assert!(wait_exit(&mut status_proc, Duration::from_secs(10)).success());
    assert_eq!(now_of(&status_pack).presence.humans, 1);
    assert_eq!(now_of(&status_pack).state_rev, 0);

    let mut mcp = spawn(&["run", &pack_arg(&mcp_pack), "--mcp", "pack-table"]);
    std::thread::sleep(Duration::from_millis(400));
    assert!(mcp.child.try_wait().unwrap().is_none(), "{}", text_of(&mcp.stderr));
    assert_eq!(now_of(&mcp_pack).presence.humans, 1);
    assert_eq!(now_of(&mcp_pack).presence.session_ids, vec!["crash-session".to_string()]);
    drop(mcp);

    let mut migrated = spawn(&["migrate", &pack_arg(&migrate_pack)]);
    assert!(wait_exit(&mut migrated, Duration::from_secs(10)).success(), "{}", text_of(&migrated.stderr));
    let now = now_of(&migrate_pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.presence.session_ids, vec!["crash-session".to_string()]);

    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let cleared = now_of(&pack);
    assert_eq!(cleared.presence.humans, 0);
    assert!(cleared.presence.session_ids.is_empty());
    assert!(cleared.presence.actors_present.is_empty());
    assert_eq!(cleared.state_rev, 1);
    assert_eq!(cleared.tick_index, 0);
    assert_eq!(cleared.world_time, 1080);
    assert!(tick_files(&pack).is_empty(), "startup clear wrote a tick file");
    assert_eq!(cleared.last_tick_at, "2026-01-01T00:00:00Z");

    let mut player = spawn(&["play", &pack_arg(&pack)]);
    wait_has(&player.stdout, "\"kind\":\"perceive\"", Duration::from_secs(5));
    assert_eq!(now_of(&pack).presence.humans, 1);
    player.stdin.take();
    assert_eq!(wait_exit(&mut player, Duration::from_secs(5)).code(), Some(0));
    let after_player = wait_humans(&pack, 0, Duration::from_secs(5));
    assert_eq!(after_player.tick_index, 0);
    assert!(tick_files(&pack).is_empty());

    write_stdin(&mut server, "stop");
    assert_eq!(wait_exit(&mut server, Duration::from_secs(5)).code(), Some(0));
    spookery::schedule::wake(Path::new(&pack_arg(&pack))).unwrap_or_else(|err| panic!("wake: {err}"));
    let caught = now_of(&pack);
    assert_eq!(caught.presence.humans, 0);
    assert_eq!(caught.tick_index, 12);
    assert_eq!(caught.world_time, 1800);
    let _ = fs::remove_dir_all(&pack);
    let _ = fs::remove_dir_all(&untouched);
    let _ = fs::remove_dir_all(&status_pack);
    let _ = fs::remove_dir_all(&mcp_pack);
    let _ = fs::remove_dir_all(&migrate_pack);
}

fn write_crashed(pack: &Path, every: Option<&str>) {
    if let Some(seconds) = every {
        let path = pack.join("rules/tick.yaml");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(
            &path,
            text.replace("tick_every_real_seconds: 300", &format!("tick_every_real_seconds: {seconds}")),
        )
        .unwrap();
    }
    let text = fs::read_to_string(pack.join("state/now.yaml")).unwrap();
    let mut now = spookery_kernel::codec::load_now(&text).unwrap();
    now.pack_rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    now.presence.humans = 1;
    now.presence.session_ids = vec!["crash-session".to_string()];
    now.presence.actors_present = vec!["pc_rowan".to_string()];
    fs::write(pack.join("state/now.yaml"), spookery_kernel::codec::emit_now(&now).unwrap()).unwrap();
}

#[test]
fn gm_tools_call_is_not_in_spec() {
    let _lock = lock_port();
    let pack = temp_pack("tools");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|err| panic!("runtime: {err}"));
    runtime.block_on(async {
        let mut stream = tokio::net::TcpStream::connect("127.0.0.1:9737")
            .await
            .unwrap_or_else(|err| panic!("connect: {err}"));
        stream.write_all(b"gm\n").await.unwrap();
        let running = ()
            .serve(stream)
            .await
            .unwrap_or_else(|err| panic!("mcp: {err}"));
        let listed = tokio::time::timeout(Duration::from_secs(10), running.list_tools(None))
            .await
            .unwrap_or_else(|_| panic!("tools/list timed out"))
            .unwrap_or_else(|err| panic!("tools/list: {err}"));
        let names: Vec<&str> = listed.tools.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                "session_open",
                "session_status",
                "perceive",
                "intent",
                "say",
                "resolve_check",
                "apply_effect",
                "npc_act",
                "commit_turn",
                "recall",
                "session_save",
            ]
        );
        for name in ["session_open", "say", "perceive", "npc_act", "patch_entity", "commit_turn"] {
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                running.call_tool(CallToolRequestParam {
                    name: (*name).into(),
                    arguments: None,
                }),
            )
            .await
            .unwrap_or_else(|_| panic!("{name} timed out"))
            .unwrap_or_else(|err| panic!("{name}: {err}"));
            let value = result.structured_content.unwrap_or_else(|| panic!("{name} has no object"));
            assert_eq!(value.get("code").and_then(|code| code.as_str()), Some("NOT_IN_SPEC"), "{name}");
        }
    });
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    assert!(tick_files(&pack).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}
