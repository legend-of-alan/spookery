use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rmcp::model::CallToolRequestParam;
use rmcp::ServiceExt;
use tokio::io::AsyncWriteExt;

use super::support::{
    copy_dir, lock_port, pack_arg, spawn, temp_pack, text_of, wait_count, wait_exit, wait_has,
    wait_listen, write_stdin, Proc,
};

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes =
                    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
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

fn refuse_line(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let n = reader.read_line(&mut line).unwrap();
    assert!(n > 0, "connection closed before the refused line");
    let mut extra = [0u8; 1];
    assert_eq!(
        reader.read(&mut extra).unwrap(),
        0,
        "shard left the connection open"
    );
    line
}

#[test]
fn v01_p14_stdin_is_not_a_play_door() {
    let _lock = lock_port();
    let pack = temp_pack("stdin");
    let before = files(&pack);
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    write_stdin(&mut server, "wait");
    let stderr = wait_has(&server.stderr, "NOT_IN_SPEC", Duration::from_secs(5));
    assert_eq!(stderr.trim(), "NOT_IN_SPEC");
    assert!(text_of(&server.stdout).is_empty());
    assert!(
        server.child.try_wait().unwrap().is_none(),
        "wait exited the process"
    );
    assert_eq!(files(&pack), before, "stdin wait wrote the pack");
    assert!(tick_files(&pack).is_empty());
    write_stdin(&mut server, "stop");
    let status = wait_exit(&mut server, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    assert_eq!(files(&pack), before, "stop with no seat wrote the pack");
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_run_as_is_not_in_spec() {
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
fn v01_p13_bind_other_than_loopback_is_not_in_spec() {
    let _lock = lock_port();
    let pack = temp_pack("bind");
    let before = files(&pack);
    for addr in ["0.0.0.0", "localhost"] {
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
    let mut server = spawn(&["run", &pack_arg(&pack), "--bind", "127.0.0.1:9737"]);
    wait_listen(&mut server);
    write_stdin(&mut server, "stop");
    assert_eq!(
        wait_exit(&mut server, Duration::from_secs(5)).code(),
        Some(0)
    );
    let mut other = spawn(&["run", &pack_arg(&pack), "--bind", "127.0.0.1:9738"]);
    let started = Instant::now();
    loop {
        if TcpStream::connect("127.0.0.1:9738").is_ok() {
            break;
        }
        if let Some(status) = other.child.try_wait().unwrap() {
            panic!(
                "second port exited {:?}\n{}",
                status.code(),
                text_of(&other.stderr)
            );
        }
        if started.elapsed() > Duration::from_secs(10) {
            panic!("127.0.0.1:9738 did not listen\n{}", text_of(&other.stderr));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    write_stdin(&mut other, "stop");
    assert_eq!(
        wait_exit(&mut other, Duration::from_secs(5)).code(),
        Some(0)
    );
    assert_port_is_free();
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p13_bind_in_use_is_the_operating_system_error() {
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
fn v01_p19_clients_without_a_listener_use_the_operating_system_error() {
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
        vec![
            "gm".to_string(),
            pack_arg(&only_yaml),
            "--stdio".to_string(),
        ],
        vec![
            "play".to_string(),
            pack_arg(&pack),
            "--as".to_string(),
            "player:pc_rowan".to_string(),
        ],
    ] {
        let owned: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut proc = spawn(&owned);
        let status = wait_exit(&mut proc, Duration::from_secs(5));
        assert_eq!(status.code(), Some(1), "{args:?}");
        assert!(text_of(&proc.stdout).is_empty(), "{args:?}");
        assert_eq!(
            text_of(&proc.stderr).trim(),
            expected.to_string(),
            "{args:?}"
        );
        assert!(!text_of(&proc.stderr).contains("NOT_IN_SPEC"), "{args:?}");
    }
    let mut gm = spawn(&["gm", &pack_arg(&pack)]);
    let status = wait_exit(&mut gm, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert!(text_of(&gm.stdout).is_empty());
    assert_eq!(text_of(&gm.stderr).trim(), "NOT_IN_SPEC");
    assert_eq!(files(&pack), before);
    let _ = fs::remove_dir_all(&pack);
    let _ = fs::remove_dir_all(&only_yaml);
}

#[test]
fn v01_play_as_mira_is_schema_and_writes_nothing() {
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
fn v01_p03_player_hello_opens_one_session() {
    let _lock = lock_port();
    let pack = temp_pack("open");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let client = spawn(&["play", &pack_arg(&pack)]);
    let stdout = wait_has(
        &client.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    assert!(stdout.contains("{\"kind\":\"resume\""));
    assert!(
        client.stderr.lock().unwrap().is_empty(),
        "{}",
        text_of(&client.stderr)
    );
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.presence.actors_present, vec!["pc_rowan".to_string()]);
    assert_eq!(now.state_rev, 1);
    assert_eq!(now.tick_index, 0);
    assert_eq!(now.world_time, 1080);
    assert!(tick_files(&pack).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p01_second_player_is_session_full() {
    let _lock = lock_port();
    let pack = temp_pack("full");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let _first = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &_first.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
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
fn v01_p02_p04_gm_connect_does_not_write_presence_and_a_second_is_full() {
    let _lock = lock_port();
    let pack = temp_pack("gm");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let before = fs::read(pack.join("state/now.yaml")).unwrap();
    let mut gm = spawn(&["gm", &pack_arg(&pack), "--stdio"]);
    assert_eq!(
        wait_for_session_full(&mut gm),
        "{\"kind\":\"refused\",\"code\":\"SESSION_FULL\"}\n"
    );
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    assert_eq!(now_of(&pack).presence.humans, 0);

    let mut second = spawn(&["gm", &pack_arg(&pack)]);
    let status = wait_exit(&mut second, Duration::from_secs(5));
    assert_eq!(status.code(), Some(1));
    assert_eq!(text_of(&second.stderr).trim(), "NOT_IN_SPEC");
    assert!(text_of(&second.stdout).is_empty());
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p05_disconnect_and_close_drop_the_player_seat_without_a_tick() {
    let _lock = lock_port();
    let pack = temp_pack("drop");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &client.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    assert_eq!(now_of(&pack).presence.humans, 1);
    client.stdin.take();
    let status = wait_exit(&mut client, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    let dropped = wait_humans(&pack, 0, Duration::from_secs(5));
    assert_eq!(dropped.state_rev, 2);
    assert_eq!(dropped.tick_index, 0);
    assert_eq!(dropped.world_time, 1080);
    assert!(dropped.presence.actors_present.is_empty());
    assert!(tick_files(&pack).is_empty());

    let mut again = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &again.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    write_stdin(&mut again, "close");
    let status = wait_exit(&mut again, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    let closed = wait_humans(&pack, 0, Duration::from_secs(5));
    assert_eq!(closed.state_rev, 4);
    assert!(tick_files(&pack).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}

// Read the live file. `open_shard` runs `recover`, which deletes `state/.txn`
// while the seat process is still inside `write_commit`.
fn wait_humans(pack: &Path, humans: u32, timeout: Duration) -> spookery_kernel::model::Now {
    let start = Instant::now();
    let path = pack.join("state/now.yaml");
    loop {
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                if let Ok(now) = spookery_kernel::codec::load_now(text) {
                    if now.presence.humans == humans {
                        return now;
                    }
                }
            }
        }
        if start.elapsed() > timeout {
            panic!("humans did not become {humans}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn v01_p14_stop_and_eof_drop_a_held_seat_and_do_not_tick() {
    let _lock = lock_port();
    let pack = temp_pack("stop");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &client.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    assert_eq!(now_of(&pack).state_rev, 1);
    write_stdin(&mut server, "stop");
    let status = wait_exit(&mut server, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 0);
    assert!(now.presence.actors_present.is_empty());
    assert_eq!(now.state_rev, 2);
    assert_eq!(now.tick_index, 0);
    assert_eq!(now.world_time, 1080);
    assert!(tick_files(&pack).is_empty());

    let pack = temp_pack("eof");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &client.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
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
    let mut gm = spawn(&["gm", &pack_arg(&pack), "--stdio"]);
    assert!(wait_for_session_full(&mut gm).contains("SESSION_FULL"));
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    write_stdin(&mut server, "stop");
    let status = wait_exit(&mut server, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p15_bad_hellos_are_schema() {
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
        assert_eq!(
            fs::read(pack.join("state/now.yaml")).unwrap(),
            before,
            "{hello:?}"
        );
    }
    assert_eq!(now_of(&pack).presence.humans, 0);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_player_lines_do_not_commit() {
    let _lock = lock_port();
    let pack = temp_pack("lines");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &client.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();
    write_stdin(&mut client, "attempt sneak shadow");
    let stderr = wait_has(&client.stderr, "NOT_IN_SPEC", Duration::from_secs(5));
    assert!(stderr.contains("NOT_IN_SPEC"));
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);
    assert!(client.child.try_wait().unwrap().is_none());
    assert!(tick_files(&pack).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p17_quiet_socket_stays_seated() {
    let _lock = lock_port();
    let pack = temp_pack("quiet");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut client = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &client.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(1));
    assert!(started.elapsed() < Duration::from_secs(30));
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.presence.actors_present, vec!["pc_rowan".to_string()]);
    assert_eq!(now.state_rev, 1);
    assert!(tick_files(&pack).is_empty());
    assert!(client.child.try_wait().unwrap().is_none());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p17_simulated_read_error_drops_the_seat_like_close() {
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
fn v01_p16_crashed_presence_clears_before_accept_and_other_commands_do_not() {
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
    assert_eq!(
        opened.now().presence.actors_present,
        vec!["pc_rowan".to_string()]
    );
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
    assert!(
        mcp.child.try_wait().unwrap().is_none(),
        "{}",
        text_of(&mcp.stderr)
    );
    assert_eq!(now_of(&mcp_pack).presence.humans, 1);
    drop(mcp);

    let mut migrated = spawn(&["migrate", &pack_arg(&migrate_pack)]);
    assert!(
        wait_exit(&mut migrated, Duration::from_secs(10)).success(),
        "{}",
        text_of(&migrated.stderr)
    );
    let now = now_of(&migrate_pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.presence.actors_present, vec!["pc_rowan".to_string()]);

    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let cleared = now_of(&pack);
    assert_eq!(cleared.presence.humans, 0);
    assert!(cleared.presence.actors_present.is_empty());
    assert!(!fs::read_to_string(pack.join("state/now.yaml"))
        .unwrap()
        .contains("session_ids"));
    assert_eq!(cleared.state_rev, 1);
    assert_eq!(cleared.tick_index, 0);
    assert_eq!(cleared.world_time, 1080);
    assert!(
        tick_files(&pack).is_empty(),
        "startup clear wrote a tick file"
    );
    assert_eq!(cleared.last_tick_at, "2026-01-01T00:00:00Z");

    let mut player = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    assert_eq!(now_of(&pack).presence.humans, 1);
    player.stdin.take();
    assert_eq!(
        wait_exit(&mut player, Duration::from_secs(5)).code(),
        Some(0)
    );
    let after_player = wait_humans(&pack, 0, Duration::from_secs(5));
    assert_eq!(after_player.tick_index, 0);
    assert!(tick_files(&pack).is_empty());

    write_stdin(&mut server, "stop");
    assert_eq!(
        wait_exit(&mut server, Duration::from_secs(5)).code(),
        Some(0)
    );
    let mut caught_shard =
        spookery_kernel::open_shard(Path::new(&pack_arg(&pack))).unwrap_or_else(|err| {
            panic!("open: {err}");
        });
    spookery::schedule::wake_shard(&mut caught_shard).unwrap_or_else(|err| panic!("wake: {err}"));
    drop(caught_shard);
    let caught = now_of(&pack);
    assert_eq!(caught.presence.humans, 0);
    assert_eq!(caught.tick_index, 12);
    assert_eq!(caught.world_time, 1800);

    let schema_pack = temp_pack("crash-schema");
    plant_session_ids(&schema_pack);
    let schema_before = fs::read(schema_pack.join("state/now.yaml")).unwrap();
    assert!(std::str::from_utf8(&schema_before)
        .unwrap()
        .contains("session_ids"));
    match spookery_kernel::open_shard(&schema_pack) {
        Err(spookery_kernel::Reason::Schema) => {}
        Err(err) => panic!("open: {err}"),
        Ok(_) => panic!("session_ids loaded"),
    }
    for args in [
        vec!["tick".to_string(), pack_arg(&schema_pack)],
        vec!["status".to_string(), pack_arg(&schema_pack)],
        vec!["migrate".to_string(), pack_arg(&schema_pack)],
        vec![
            "run".to_string(),
            pack_arg(&schema_pack),
            "--mcp".to_string(),
            "pack-table".to_string(),
        ],
    ] {
        let owned: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut proc = spawn(&owned);
        let status = wait_exit(&mut proc, Duration::from_secs(10));
        assert_eq!(
            status.code(),
            Some(1),
            "{args:?}\n{}",
            text_of(&proc.stderr)
        );
        assert!(
            text_of(&proc.stderr).contains("SCHEMA"),
            "{args:?}\n{}",
            text_of(&proc.stderr)
        );
        assert_eq!(
            fs::read(schema_pack.join("state/now.yaml")).unwrap(),
            schema_before,
            "{args:?}"
        );
    }
    let _ = fs::remove_dir_all(&schema_pack);
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
            text.replace(
                "tick_every_real_seconds: 300",
                &format!("tick_every_real_seconds: {seconds}"),
            ),
        )
        .unwrap();
    }
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

fn plant_session_ids(pack: &Path) {
    let path = pack.join("state/now.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let from = "presence:\n  humans: 0\n  actors_present: []\n";
    let to = "presence:\n  humans: 1\n  session_ids: [\"crash-session\"]\n  actors_present: [\"pc_rowan\"]\n";
    assert!(text.contains(from), "{text}");
    fs::write(&path, text.replacen(from, to, 1)).unwrap();
}

#[test]
fn v01_gm_tools_call_is_not_in_spec() {
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
        let running = ().serve(stream).await.unwrap_or_else(|err| panic!("mcp: {err}"));
        let listed = tokio::time::timeout(Duration::from_secs(10), running.list_tools(None))
            .await
            .unwrap_or_else(|_| panic!("tools/list timed out"))
            .unwrap_or_else(|err| panic!("tools/list: {err}"));
        let names: Vec<&str> = listed.tools.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                "session_status",
                "perceive",
                "intent",
                "say",
                "npc_act",
                "commit_turn",
            ]
        );
        for (name, code) in [
            ("session_open", "NOT_IN_SPEC"),
            ("say", "SCHEMA"),
            ("perceive", "UNSCOPED_PERCEIVE"),
            ("npc_act", "LLM_DISABLED"),
            ("patch_entity", "NOT_IN_SPEC"),
        ] {
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
            let value = result
                .structured_content
                .unwrap_or_else(|| panic!("{name} has no object"));
            assert_eq!(
                value.get("code").and_then(|code| code.as_str()),
                Some(code),
                "{name}"
            );
        }
        let commit = tokio::time::timeout(
            Duration::from_secs(10),
            running.call_tool(CallToolRequestParam {
                name: "commit_turn".into(),
                arguments: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("commit_turn timed out"))
        .unwrap_or_else(|err| panic!("commit_turn: {err}"));
        let value = commit.structured_content.unwrap();
        assert_eq!(
            value.get("code").and_then(|code| code.as_str()),
            Some("SCHEMA")
        );
        let intent = tokio::time::timeout(
            Duration::from_secs(10),
            running.call_tool(CallToolRequestParam {
                name: "intent".into(),
                arguments: Some(
                    serde_json::json!({
                        "actor_id": "pc_rowan",
                        "actor_kind": "player",
                        "source": "player",
                        "move": "wait",
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("intent timed out"))
        .unwrap_or_else(|err| panic!("intent: {err}"));
        let value = intent.structured_content.unwrap();
        assert!(value
            .get("queued")
            .and_then(|id| id.as_str())
            .is_some_and(|id| !id.is_empty()));
        assert!(value.get("code").is_none());
    });
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), before);
    assert!(tick_files(&pack).is_empty());
    assert!(!pack.join("state/log/session.jsonl").exists());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p06_say_while_gm_is_seated_appends_one_tape_line_without_a_tick() {
    let _lock = lock_port();
    let pack = temp_pack("say");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut gm = spawn(&["gm", &pack_arg(&pack), "--stdio"]);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        gm.child.try_wait().unwrap().is_none(),
        "gm seat was not held"
    );
    let mut player = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();
    let now = now_of(&pack);
    let tape = format!(
        "{{\"seq\":1,\"from\":\"player:pc_rowan\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[\"pc_rowan\"],\"text\":\"hello\",\"tick_index\":{},\"world_time\":{}}}\n",
        now.tick_index, now.world_time
    );
    write_stdin(&mut player, "say hello");
    let stdout = wait_has(&player.stdout, "\"text\":\"hello\"", Duration::from_secs(5));
    assert!(stdout.contains(
        "{\"kind\":\"say\",\"from\":\"player:pc_rowan\",\"address\":\"table\",\"text\":\"hello\"}"
    ));
    assert!(!text_of(&player.stderr).contains("NOT_IN_SPEC"));
    assert_eq!(
        fs::read_to_string(pack.join("state/log/session.jsonl")).unwrap(),
        tape
    );
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);
    let after = now_of(&pack);
    assert_eq!(after.state_rev, now.state_rev);
    assert_eq!(after.world_time, now.world_time);
    assert_eq!(after.tick_index, now.tick_index);
    assert!(tick_files(&pack).is_empty());
    assert!(text_of(&server.stdout).contains(tape.trim_end()));
    assert!(
        gm.child.try_wait().unwrap().is_none(),
        "gm seat dropped on say"
    );
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p07_actor_say_is_on_the_gm_and_speaker_cursors_and_absent_for_a_later_actor() {
    let _lock = lock_port();
    let pack = temp_pack("actor-say");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let gm = spawn(&["gm", &pack_arg(&pack), "--stdio"]);
    std::thread::sleep(Duration::from_millis(200));
    let mut player = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();
    let now = now_of(&pack);
    write_stdin(&mut player, "talk pc_rowan psst");
    let player_event =
        "{\"kind\":\"say\",\"from\":\"player:pc_rowan\",\"address\":\"actor\",\"text\":\"psst\"}";
    let gm_event = "{\"kind\":\"say\",\"seq\":1,\"from\":\"player:pc_rowan\",\"address\":\"actor\",\"place\":null,\"actor\":\"pc_rowan\",\"recipients\":[\"pc_rowan\"],\"text\":\"psst\"}";
    let player_out = wait_has(&player.stdout, player_event, Duration::from_secs(5));
    assert!(player_out.contains(player_event));
    let gm_out = wait_has(&gm.stdout, gm_event, Duration::from_secs(5));
    assert!(gm_out.contains(gm_event));
    assert!(!text_of(&player.stderr).contains("NOT_IN_SPEC"));
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);
    assert_eq!(now_of(&pack).world_time, now.world_time);
    assert!(tick_files(&pack).is_empty());

    player.stdin.take();
    assert_eq!(
        wait_exit(&mut player, Duration::from_secs(5)).code(),
        Some(0)
    );
    let freed = wait_humans(&pack, 0, Duration::from_secs(5));
    assert_eq!(freed.state_rev, now.state_rev + 1);
    assert_eq!(freed.world_time, now.world_time);
    assert!(tick_files(&pack).is_empty());

    let speaker = "pc_rowan";
    let later = "pc_later";
    assert_ne!(speaker, later);
    let speaker_cursor = spookery::seat::player_scrollback(&pack, speaker).unwrap();
    let later_cursor = spookery::seat::player_scrollback(&pack, later).unwrap();
    assert!(speaker_cursor
        .iter()
        .any(|line| line.contains(player_event)));
    assert!(later_cursor.iter().all(|line| !line.contains("psst")));
    assert!(later_cursor.is_empty());
    let _ = fs::remove_dir_all(&pack);
    let _ = server;
}

#[test]
fn v01_p08_talk_appends_for_a_seated_player_and_live_only_when_no_human() {
    let pack = temp_pack("talk-direct");
    let shard = spookery_kernel::open_shard(&pack).unwrap();
    assert_eq!(shard.now().presence.humans, 0);
    let err = spookery::seat::speak(&shard, "pc_rowan", "talk mira good evening").unwrap_err();
    assert_eq!(err, spookery_kernel::Reason::TableEmpty);
    assert!(!pack.join("state/log/session.jsonl").exists());
    drop(shard);

    let _lock = lock_port();
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut player = spawn(&["play", &pack_arg(&pack)]);
    wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let seated = fs::read(pack.join("state/now.yaml")).unwrap();
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 1);
    write_stdin(&mut player, "talk mira good evening");
    let event = "{\"kind\":\"say\",\"from\":\"player:pc_rowan\",\"address\":\"actor\",\"text\":\"good evening\"}";
    let stdout = wait_has(&player.stdout, event, Duration::from_secs(5));
    assert!(stdout.contains(event));
    assert!(!text_of(&player.stderr).contains("NOT_IN_SPEC"));
    assert!(!text_of(&player.stderr).contains("TABLE_EMPTY"));
    let tape = format!(
        "{{\"seq\":1,\"from\":\"player:pc_rowan\",\"address\":\"actor\",\"place\":null,\"actor\":\"mira\",\"recipients\":[\"pc_rowan\"],\"text\":\"good evening\",\"tick_index\":{},\"world_time\":{}}}\n",
        now.tick_index, now.world_time
    );
    assert_eq!(
        fs::read_to_string(pack.join("state/log/session.jsonl")).unwrap(),
        tape
    );
    assert_eq!(fs::read(pack.join("state/now.yaml")).unwrap(), seated);
    assert!(tick_files(&pack).is_empty());
    let _ = fs::remove_dir_all(&pack);
    let _ = server;
}

#[test]
fn v01_torn_tape_tail_is_not_a_record() {
    let pack = temp_pack("torn");
    let log = pack.join("state/log");
    fs::create_dir_all(&log).unwrap();
    fs::write(log.join("session.jsonl"), b"{\"seq\":9,\"torn").unwrap();
    let shard = spookery_kernel::open_shard(&pack).unwrap();
    assert_eq!(shard.now().presence.humans, 0);
    let spoken = spookery::seat::speak(&shard, "pc_rowan", "say hello").unwrap();
    assert!(spoken.tape.starts_with("{\"seq\":1,"));
    let text = fs::read_to_string(log.join("session.jsonl")).unwrap();
    assert_eq!(text, format!("{}\n", spoken.tape));
    assert!(!text.contains("torn"));
    assert_eq!(now_of(&pack).state_rev, 0);
    assert_eq!(now_of(&pack).world_time, shard.now().world_time);
    assert!(tick_files(&pack).is_empty());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p10_no_gm_wait_commits_one_live_tick() {
    let _lock = lock_port();
    let pack = temp_pack("live-wait");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut player = spawn(&["play", &pack_arg(&pack)]);
    let hello = wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let seated = now_of(&pack);
    assert_eq!(seated.presence.humans, 1);
    assert_eq!(seated.world_time, 1080);
    assert_eq!(seated.tick_index, 0);
    assert!(tick_files(&pack).is_empty());
    write_stdin(&mut player, "wait");
    let stdout = wait_count(
        &player.stdout,
        "\"kind\":\"resume\"",
        hello.matches("\"kind\":\"resume\"").count() + 1,
        Duration::from_secs(5),
    );
    let added = &stdout[hello.len()..];
    assert!(added.contains("\"kind\":\"resume\""), "{added}");
    assert!(!added.contains("\"kind\":\"queued\""), "{added}");
    assert!(!text_of(&player.stderr).contains("NOT_IN_SPEC"));
    let now = now_of(&pack);
    assert_eq!(now.presence.humans, 1);
    assert_eq!(now.world_time, 1085);
    assert_ne!(now.world_time, 1080 + 60);
    assert_eq!(now.tick_index, 1);
    assert_eq!(now.state_rev, seated.state_rev + 1);
    assert_ne!(now.last_tick_at, seated.last_tick_at);
    assert_eq!(tick_files(&pack), vec!["tick-0001.jsonl".to_string()]);
    let tick: serde_json::Value = serde_json::from_str(
        fs::read_to_string(pack.join("state/log/tick-0001.jsonl"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert_eq!(tick["presence_humans"], 1);
    assert_eq!(tick["world_time_before"], 1080);
    assert_eq!(tick["world_time_after"], 1085);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p10_p11_wake_while_a_seat_is_held_does_not_tick_or_drain() {
    let pack = temp_pack("held-wake");
    let mut shard = spookery_kernel::open_shard(&pack).unwrap();
    let session = spookery::seat::seat_player(&mut shard, "pc_rowan").unwrap();
    assert!(!spookery::seat::on_scheduled_wake(&mut shard, true, false).unwrap());
    assert_eq!(shard.now().presence.humans, 1);
    assert_eq!(shard.now().world_time, 1080);
    assert_eq!(shard.now().tick_index, 0);
    assert!(tick_files(&pack).is_empty());

    let note = spookery::seat::queue_wait(&mut shard, "pc_rowan").unwrap();
    assert_eq!(shard.queued_intents().len(), 1);
    assert!(!spookery::seat::on_scheduled_wake(&mut shard, true, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.queued_intents()[0].id, note.id);
    assert_eq!(shard.now().world_time, 1080);
    assert!(tick_files(&pack).is_empty());

    assert!(spookery::seat::release_player_seat(
        &mut shard,
        &session,
        "pc_rowan",
        spookery::seat::SocketEnd::Close
    )
    .unwrap());
    assert_eq!(shard.now().presence.humans, 0);
    assert_eq!(shard.now().world_time, 1080);
    assert!(!spookery::seat::on_scheduled_wake(&mut shard, false, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.now().world_time, 1080);
    assert_eq!(shard.now().tick_index, 0);
    assert!(tick_files(&pack).is_empty());
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p11_gm_commit_advances_world_time_and_a_held_wake_does_not() {
    let _lock = lock_port();
    let pack = temp_pack("gm-commit");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let mut stream = tokio::net::TcpStream::connect("127.0.0.1:9737")
            .await
            .unwrap();
        stream.write_all(b"gm\n").await.unwrap();
        let running = ().serve(stream).await.unwrap();
        let mut player = spawn(&["play", &pack_arg(&pack)]);
        wait_has(
            &player.stdout,
            "\"kind\":\"perceive\"",
            Duration::from_secs(5),
        );
        let seated = now_of(&pack);
        assert_eq!(seated.presence.humans, 1);
        assert_eq!(seated.world_time, 1080);
        let empty = call_commit(&running, true).await;
        assert!(empty.get("log_path").unwrap().is_null(), "{empty}");
        assert_eq!(empty["diff_summary"].as_array().unwrap().len(), 0);
        assert_eq!(now_of(&pack).world_time, 1080);
        assert_eq!(now_of(&pack).state_rev, seated.state_rev);
        assert!(tick_files(&pack).is_empty());

        let before = text_of(&player.stdout);
        write_stdin(&mut player, "wait");
        let stdout = wait_has(
            &player.stdout,
            "\"kind\":\"queued\"",
            Duration::from_secs(5),
        );
        let added = &stdout[before.len()..];
        assert!(added.contains("\"kind\":\"queued\""));
        assert!(!added.contains("\"kind\":\"resume\""), "{added}");
        let queued = queued_id(&stdout);
        assert_eq!(now_of(&pack).world_time, 1080);
        assert!(tick_files(&pack).is_empty());

        let committed = call_commit(&running, true).await;
        assert!(committed.get("code").is_none(), "{committed}");
        assert!(committed["log_path"]
            .as_str()
            .unwrap()
            .ends_with("tick-0001.jsonl"));
        let now = now_of(&pack);
        assert_eq!(now.world_time, 1085);
        assert_ne!(now.world_time, 1140);
        assert_eq!(now.tick_index, 1);
        assert_eq!(now.presence.humans, 1);
        assert_eq!(now.state_rev, seated.state_rev + 1);
        assert_ne!(now.last_tick_at, seated.last_tick_at);
        let tick = tick_value(&pack);
        let waits = player_waits(&tick);
        assert_eq!(waits, vec![queued]);
        assert_eq!(tick["presence_humans"], 1);
        assert_eq!(tick["world_time_after"], 1085);
        let _ = player;
        let _ = running;
    });
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn v01_p18_early_wake_after_commit_does_not_tick_and_a_due_wake_does() {
    let schedule =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cli/schedule.rs"))
            .unwrap();
    assert!(!schedule.contains("pub fn wake("));
    assert!(schedule.contains("pub fn wake_shard("));
    assert!(!schedule.contains("player_held"));
    assert!(!schedule.contains("gm_held"));
    let turn =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cli/turn.rs"))
            .unwrap();
    assert!(turn.contains("schedule::wake_shard"));
    assert!(!turn.contains("open_shard"));

    let pack = temp_pack("stamp-wake");
    let mut shard = spookery_kernel::open_shard(&pack).unwrap();
    let session = spookery::seat::seat_player(&mut shard, "pc_rowan").unwrap();
    spookery::seat::queue_wait(&mut shard, "pc_rowan").unwrap();
    assert!(spookery::seat::commit_queued(&mut shard).unwrap());
    let committed = shard.now().clone();
    assert_eq!(committed.world_time, 1085);
    assert_eq!(committed.tick_index, 1);
    assert_eq!(committed.presence.humans, 1);
    assert_ne!(committed.last_tick_at, "2026-01-01T00:00:00Z");
    assert!(spookery::seat::release_player_seat(
        &mut shard,
        &session,
        "pc_rowan",
        spookery::seat::SocketEnd::Close
    )
    .unwrap());
    assert_eq!(shard.now().presence.humans, 0);
    assert_eq!(shard.queued_intents().len(), 0);
    assert!(!spookery::seat::on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().world_time, committed.world_time);
    assert_eq!(shard.now().tick_index, committed.tick_index);
    assert_eq!(shard.now().last_tick_at, committed.last_tick_at);
    assert_eq!(shard.now().state_rev, committed.state_rev + 1);
    assert_eq!(tick_files(&pack), vec!["tick-0001.jsonl".to_string()]);
    let _ = fs::remove_dir_all(&pack);

    let due = temp_pack("due-wake");
    let mut shard = spookery_kernel::open_shard(&due).unwrap();
    let note = spookery::seat::queue_wait(&mut shard, "pc_rowan").unwrap();
    assert_eq!(shard.now().last_tick_at, "2026-01-01T00:00:00Z");
    assert!(spookery::seat::on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().presence.humans, 0);
    assert_eq!(shard.now().tick_index, 12);
    assert_eq!(shard.now().world_time, 1800);
    assert!(shard.queued_intents().is_empty());
    let first: serde_json::Value = serde_json::from_str(
        fs::read_to_string(due.join("state/log/tick-0001.jsonl"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    let second: serde_json::Value = serde_json::from_str(
        fs::read_to_string(due.join("state/log/tick-0002.jsonl"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert_eq!(player_waits(&first), vec![note.id]);
    assert!(player_waits(&second).is_empty(), "{second}");
    assert!(!spookery::seat::on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().tick_index, 12);
    assert!(shard.queued_intents().is_empty());
    let _ = fs::remove_dir_all(&due);
}

#[test]
fn v01_p20_queued_wait_survives_gm_drop_and_the_next_wait_applies_both() {
    let _lock = lock_port();
    let pack = temp_pack("both-waits");
    let mut server = spawn(&["run", &pack_arg(&pack)]);
    wait_listen(&mut server);
    let mut gm = spawn(&["gm", &pack_arg(&pack), "--stdio"]);
    std::thread::sleep(Duration::from_millis(200));
    assert!(gm.child.try_wait().unwrap().is_none());
    let mut player = spawn(&["play", &pack_arg(&pack)]);
    let hello = wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let seated = now_of(&pack);
    assert_eq!(seated.world_time, 1080);
    assert_eq!(seated.presence.humans, 1);
    write_stdin(&mut player, "wait");
    let queued_out = wait_has(
        &player.stdout,
        "\"kind\":\"queued\"",
        Duration::from_secs(5),
    );
    let added = &queued_out[hello.len()..];
    assert!(added.contains("\"kind\":\"queued\""));
    assert_eq!(added.matches("\"kind\":\"resume\"").count(), 0, "{added}");
    let first = queued_id(&queued_out);
    assert!(!text_of(&player.stderr).contains("NOT_IN_SPEC"));
    assert_eq!(now_of(&pack).world_time, 1080);
    assert!(tick_files(&pack).is_empty());

    let said = text_of(&player.stdout);
    write_stdin(&mut player, "say before the round");
    let after_say = wait_has(&player.stdout, "before the round", Duration::from_secs(5));
    let say_added = &after_say[said.len()..];
    assert!(say_added.contains("\"kind\":\"say\""));
    assert!(!say_added.contains("\"kind\":\"resume\""), "{say_added}");
    assert_eq!(now_of(&pack).world_time, 1080);
    assert_eq!(now_of(&pack).tick_index, 0);
    assert!(tick_files(&pack).is_empty());
    assert!(pack.join("state/log/session.jsonl").is_file());

    gm.stdin.take();
    assert_eq!(wait_exit(&mut gm, Duration::from_secs(5)).code(), Some(0));
    let dropped = now_of(&pack);
    assert_eq!(dropped.world_time, 1080);
    assert_eq!(dropped.tick_index, 0);
    assert_eq!(dropped.presence.humans, 1);
    assert!(tick_files(&pack).is_empty());

    write_stdin(&mut player, "wait");
    let stderr = wait_has(&player.stderr, "SCHEMA", Duration::from_secs(5));
    assert_eq!(stderr.trim(), "SCHEMA");
    let now = now_of(&pack);
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.tick_index, 0);
    assert_eq!(now.presence.humans, 1);
    assert!(tick_files(&pack).is_empty());
    let _ = first;
    let _ = fs::remove_dir_all(&pack);
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
