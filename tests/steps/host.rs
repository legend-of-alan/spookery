use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cucumber::{then, when};
use serde_json::json;
use spookery_ai::{drive, parse_tool_line, CompleteError, Script, Seat, Watch};

use crate::support::{self, pack_arg, temp_pack};
use crate::World;

use super::table;

#[when(expr = "the operator runs ai-gm with no pack")]
fn the_operator_runs_host_with_no_pack(world: &mut World) {
    table::run(world, &["ai-gm"]);
}

#[when(expr = "the operator runs ai-gm with an extra argument")]
fn the_operator_runs_host_with_an_extra_argument(world: &mut World) {
    let pack = pack_arg(table::pack(world));
    table::run(world, &["ai-gm", &pack, "extra"]);
}

#[when(expr = "the operator runs ai-gm with nothing listening")]
fn the_operator_runs_host_with_nothing_listening(world: &mut World) {
    let _port = hold_9737();
    let pack = pack_arg(table::pack(world));
    let output = Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["ai-gm", &pack])
        .env("XAI_API_KEY", "test-key")
        .env("SPOOK_GM_MODEL", "grok-test")
        .env_remove("SPOOK_GM_URL")
        .output()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[when(expr = "the operator runs ai-gm without the model setting")]
fn the_operator_runs_host_without_the_model_setting(world: &mut World) {
    let listener =
        TcpListener::bind("127.0.0.1:9737").unwrap_or_else(|err| panic!("bind 9737: {err}"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|err| panic!("nonblocking: {err}"));
    let pack = pack_arg(table::pack(world));
    let output = Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["ai-gm", &pack])
        .env("XAI_API_KEY", "test-key")
        .env_remove("SPOOK_GM_MODEL")
        .output()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    match listener.accept() {
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("9737 accepted a connection: {other:?}"),
    }
    drop(listener);
    let listener =
        TcpListener::bind("127.0.0.1:9737").unwrap_or_else(|err| panic!("bind 9737: {err}"));
    listener.set_nonblocking(true).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["ai-gm", &pack])
        .env_remove("XAI_API_KEY")
        .env("SPOOK_GM_MODEL", "grok-test")
        .output()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("XAI_API_KEY is unset"), "{stderr}");
    match listener.accept() {
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("9737 accepted a connection: {other:?}"),
    }
}

#[then(expr = "the listener on 9737 accepted no connection")]
fn the_listener_accepted_no_connection(world: &mut World) {
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    assert!(
        world.stderr.contains("SPOOK_GM_MODEL is unset"),
        "{}",
        world.stderr
    );
    assert!(world.stdout.is_empty(), "{}", world.stdout);
}

#[when(expr = "the host seat says good evening and calls an unknown tool")]
fn the_host_seat_says(world: &mut World) {
    let port = crate::with_live(|live| live.port);
    let mark = table::player_len();
    let mut seat =
        Seat::connect(&format!("127.0.0.1:{port}")).unwrap_or_else(|err| panic!("{err}"));
    let say = seat
        .call("say", json!({"address": "table", "text": "good evening"}))
        .unwrap_or_else(|err| panic!("{err}"));
    let again = parse_tool_line(&say.raw).unwrap_or_else(|err| panic!("{err}"));
    assert!(!say.refused, "{}", say.raw);
    assert!(!again.refused, "{}", again.raw);
    assert_eq!(again.body, say.body);
    world.player_added = table::wait_player_from(mark, "{\"kind\":\"say\"");
    world.scratch = say.raw;
    let refused = seat
        .call("not_in_spec", json!({}))
        .unwrap_or_else(|err| panic!("{err}"));
    world.stdout = if refused.refused && refused.body.get("code").is_some() {
        "refused".to_string()
    } else {
        refused.raw
    };
}

#[then(expr = "the player saw a say from the host")]
fn the_player_saw_a_say(world: &mut World) {
    assert!(
        world.player_added.contains("{\"kind\":\"say\""),
        "{}",
        world.player_added
    );
}

#[then(expr = "the tape from is gm")]
fn the_tape_from_is_gm(world: &mut World) {
    let tape = fs::read_to_string(table::pack(world).join("state/log/session.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"));
    assert!(tape.contains("\"from\":\"gm\""), "{tape}");
}

#[then(expr = "the host call was refused")]
fn the_host_call_was_refused(world: &mut World) {
    assert_eq!(world.stdout, "refused", "{}", world.scratch);
}

#[when(expr = "the scripted host plays the turn checks")]
fn the_scripted_host_plays_the_turn_checks(_world: &mut World) {
    speech_leaves_world_time_and_a_gm_say_does_not_call();
    travel_ticks_once(false);
    travel_ticks_once(true);
    epilogue_does_not_tick_again();
    sit_commits_the_waiting_queue();
}

#[when(expr = "play and host and a stub run a speech turn and a travel turn")]
fn play_and_host_and_a_stub(_world: &mut World) {
    let _gate = hold_9737();
    thread::sleep(Duration::from_millis(100));
    let pack = temp_pack("host-stub");
    let aldric = pack.join("cast/aldric.md");
    let mut body = fs::read_to_string(&aldric).unwrap();
    body.push_str("\nCASTBODYONLY-FIXTURE\n");
    fs::write(&aldric, body).unwrap();
    spookery_kernel::commit::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    let (stub_port, posts) = stub_server();
    let pack_arg = pack_arg(&pack);
    let mut server = support::spawn(&["run", &pack_arg, "--bind", "127.0.0.1:9737"]);
    wait_port(9737, &mut server);
    let mut host = Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["ai-gm", &pack_arg])
        .env("XAI_API_KEY", "test-key")
        .env("SPOOK_GM_MODEL", "grok-test")
        .env("SPOOK_GM_URL", format!("http://127.0.0.1:{stub_port}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    let host_out = host.stdout.take().unwrap();
    let host_err = host.stderr.take().unwrap();
    let host_stdout = Arc::new(Mutex::new(String::new()));
    let host_stderr = Arc::new(Mutex::new(String::new()));
    drain(host_out, Arc::clone(&host_stdout));
    drain(host_err, Arc::clone(&host_stderr));
    crate::track_child(host);
    wait_text(&host_stderr, "listening");
    let mut play = support::spawn(&["play", &pack_arg]);
    wait_text(&play.stdout, "\"kind\":\"perceive\"");
    let mut play_in = play.stdin.take().unwrap();
    writeln!(play_in, "say hello").unwrap();
    play_in.flush().unwrap();
    wait_posts(&posts, 1);
    assert_eq!(world_time(&pack), 1080, "speech ticked");
    let first = posts.lock().unwrap()[0].clone();
    assert!(first.contains("act"), "{first}");
    assert!(!first.contains("CASTBODYONLY-FIXTURE"), "{first}");
    writeln!(play_in, "travel docks_street").unwrap();
    play_in.flush().unwrap();
    wait_posts(&posts, 2);
    table::wait_world_time(&pack, 1085);
    let place = table::now_of(&pack)
        .actors
        .get("pc_rowan")
        .map(|actor| actor.place().to_string());
    assert_eq!(place.as_deref(), Some("docks_street"));
    let second = posts.lock().unwrap()[1].clone();
    assert!(second.contains("act"), "{second}");
    assert!(!second.contains("CASTBODYONLY-FIXTURE"), "{second}");
    thread::sleep(Duration::from_millis(200));
    assert_eq!(world_time(&pack), 1085, "travel ticked twice");
    assert!(
        support::text_of(&host_stdout).is_empty(),
        "{}",
        support::text_of(&host_stdout)
    );
    drop(play_in);
    drop(play);
    drop(server);
    let _ = fs::remove_dir_all(&pack);
}

fn speech_leaves_world_time_and_a_gm_say_does_not_call() {
    with_host("host-speech", |pack, _port, script, watch| {
        table::seat_player(pack);
        let sit = world_time(pack);
        script.push(Ok(Some(
            r#"{"says":[{"address":"table","text":"evening"}],"intents":[]}"#.to_string(),
        )));
        table::player_send("say hello", "\"kind\":\"say\"");
        wait_idle(watch, script, 1);
        assert_eq!(world_time(pack), sit, "speech changed world_time");
        let tape = tape_text(pack);
        assert!(tape.contains("\"from\":\"gm\""), "{tape}");
        assert!(tape.contains("evening"), "{tape}");
        thread::sleep(Duration::from_millis(200));
        assert_eq!(script.calls().len(), 1, "a gm say called the model");
        assert!(!watch.busy.load(Ordering::Acquire));
        assert_eq!(world_time(pack), sit);
    });
}

fn travel_ticks_once(transport_error: bool) {
    let label = if transport_error {
        "host-transport"
    } else {
        "host-travel"
    };
    with_host(label, |pack, _port, script, watch| {
        if transport_error {
            script.push(Err(CompleteError::Transport(
                "model transport error".to_string(),
            )));
        } else {
            script.push(Ok(Some(r#"{"says":[],"intents":[]}"#.to_string())));
        }
        table::seat_player(pack);
        let sit = world_time(pack);
        table::player_send("travel docks_street", "\"kind\":\"queued\"");
        table::wait_world_time(pack, sit + 5);
        wait_idle(watch, script, 1);
        thread::sleep(Duration::from_millis(200));
        assert_eq!(world_time(pack), sit + 5, "travel ticked more than once");
        assert_eq!(script.calls().len(), 1, "{}", script.calls().len());
        assert_eq!(
            table::now_of(pack)
                .actors
                .get("pc_rowan")
                .map(|actor| actor.place().to_string()),
            Some("docks_street".to_string())
        );
    });
}

fn epilogue_does_not_tick_again() {
    with_host("host-epilogue", |pack, _port, script, watch| {
        script.push(Ok(Some(r#"{"says":[],"intents":[]}"#.to_string())));
        script.push(Ok(Some(
            r#"{"says":[{"address":"table","text":"the roll lands"}]}"#.to_string(),
        )));
        table::seat_player(pack);
        let sit = world_time(pack);
        table::player_send("grit", "\"kind\":\"queued\"");
        table::wait_world_time(pack, sit + 5);
        wait_idle(watch, script, 2);
        let tape = tape_text(pack);
        assert!(tape.contains("the roll lands"), "{tape}");
        let calls = script.calls();
        assert_eq!(calls.len(), 2, "epilogue did not run");
        assert!(!calls[0].says_only);
        assert!(calls[1].says_only);
        assert!(
            calls[1].prompt.contains("\"checks\""),
            "{}",
            calls[1].prompt
        );
        thread::sleep(Duration::from_millis(200));
        assert_eq!(world_time(pack), sit + 5, "epilogue ticked");
        assert_eq!(script.calls().len(), 2);
    });
}

fn sit_commits_the_waiting_queue() {
    let pack = temp_pack("host-sit");
    let port = table::free_port();
    let bind = format!("127.0.0.1:{port}");
    table::listen(&pack, &bind, port);
    table::seat_gm();
    table::seat_player(&pack);
    table::player_send("say good evening", "\"kind\":\"say\"");
    table::player_send("travel docks_street", "\"kind\":\"queued\"");
    assert_eq!(world_time(&pack), 1080);
    crate::with_live(|live| {
        if let Some(stream) = live.gm_write.as_mut() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        live.gm_write.take();
    });
    let script = Script::new();
    let (handle, watch) = start_after_gm(&pack, port, &script);
    table::wait_world_time(&pack, 1085);
    wait_idle(&watch, &script, 1);
    assert_eq!(
        watch.high_water.load(Ordering::Acquire),
        watch.sit_seq.load(Ordering::Acquire),
        "sit moved the high water"
    );
    assert!(watch.sit_seq.load(Ordering::Acquire) > 0);
    thread::sleep(Duration::from_millis(200));
    assert_eq!(world_time(&pack), 1085);
    assert_eq!(script.calls().len(), 1);
    stop_host(handle);
    let _ = fs::remove_dir_all(&pack);
}

fn with_host(label: &str, body: impl FnOnce(&Path, u16, &Script, &Watch)) {
    let pack = temp_pack(label);
    let port = table::free_port();
    let bind = format!("127.0.0.1:{port}");
    table::listen(&pack, &bind, port);
    let script = Script::new();
    let watch = Arc::new(Watch::default());
    let watch_bg = Arc::clone(&watch);
    let script_bg = script.clone();
    let pack_bg = pack.clone();
    let addr = format!("127.0.0.1:{port}");
    let handle = thread::spawn(move || {
        let mut script = script_bg;
        drive(&pack_bg, &addr, &mut script, &watch_bg)
    });
    wait_listening(&watch);
    body(&pack, port, &script, &watch);
    stop_host(handle);
    let _ = fs::remove_dir_all(&pack);
}

fn stop_host(handle: thread::JoinHandle<Result<(), spookery_ai::HostError>>) {
    crate::with_live(|live| {
        live.server.take();
    });
    let _ = handle.join();
}

fn start_after_gm(
    pack: &Path,
    port: u16,
    script: &Script,
) -> (
    thread::JoinHandle<Result<(), spookery_ai::HostError>>,
    Arc<Watch>,
) {
    let mut last = String::new();
    for _ in 0..40 {
        script.push(Ok(Some(r#"{"says":[],"intents":[]}"#.to_string())));
        let watch = Arc::new(Watch::default());
        let watch_bg = Arc::clone(&watch);
        let script_bg = script.clone();
        let pack_bg = pack.to_path_buf();
        let addr = format!("127.0.0.1:{port}");
        let handle = thread::spawn(move || {
            let mut script = script_bg;
            drive(&pack_bg, &addr, &mut script, &watch_bg)
        });
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(500) {
            if watch.listening.load(Ordering::Acquire) {
                return (handle, watch);
            }
            if handle.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if watch.listening.load(Ordering::Acquire) {
            return (handle, watch);
        }
        let start = Instant::now();
        while !handle.is_finished() && start.elapsed() < Duration::from_secs(2) {
            if watch.listening.load(Ordering::Acquire) {
                return (handle, watch);
            }
            thread::sleep(Duration::from_millis(20));
        }
        if watch.listening.load(Ordering::Acquire) {
            return (handle, watch);
        }
        if !handle.is_finished() {
            last = "host still dialing".to_string();
            break;
        }
        match handle.join() {
            Ok(Err(err)) => last = err.message,
            Ok(Ok(())) => last = "host exited".to_string(),
            Err(_) => last = "host thread panicked".to_string(),
        }
        thread::sleep(Duration::from_millis(50));
    }
    let stderr = crate::with_live(|live| {
        live.server
            .as_ref()
            .map(|server| support::text_of(&server.stderr))
            .unwrap_or_default()
    });
    panic!("host did not listen: {last}\n{stderr}");
}

fn wait_listening(watch: &Watch) {
    let start = Instant::now();
    while !watch.listening.load(Ordering::Acquire) {
        if start.elapsed() > Duration::from_secs(5) {
            panic!("host did not listen");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_idle(watch: &Watch, script: &Script, calls: usize) {
    let start = Instant::now();
    loop {
        let ready = script.calls().len() >= calls && !watch.busy.load(Ordering::Acquire);
        if ready {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "host still busy calls {} busy {}",
                script.calls().len(),
                watch.busy.load(Ordering::Acquire)
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn world_time(pack: &Path) -> u64 {
    let text = fs::read_to_string(pack.join("state/now.yaml")).unwrap();
    spookery_kernel::codec::load_now(&text).unwrap().world_time
}

fn tape_text(pack: &Path) -> String {
    fs::read_to_string(pack.join("state/log/session.jsonl")).unwrap_or_default()
}

fn hold_9737() -> TcpListener {
    match TcpListener::bind("127.0.0.1:9737") {
        Ok(listener) => {
            drop(listener);
            TcpListener::bind("127.0.0.1:0").unwrap()
        }
        Err(err) => panic!("9737 is in use: {err}"),
    }
}

fn wait_port(port: u16, server: &mut support::Proc) {
    let start = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    loop {
        if TcpStream::connect(&addr).is_ok() {
            return;
        }
        if let Some(status) = server.child.try_wait().unwrap() {
            panic!(
                "spook run exited {status:?}\n{}",
                support::text_of(&server.stderr)
            );
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("9737 did not open");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn stub_server() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let posts = Arc::new(Mutex::new(Vec::new()));
    let slot = Arc::clone(&posts);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            let body = read_http(&mut stream);
            slot.lock().unwrap().push(body);
            let resp = r#"{"id":"stub","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"prose","tool_calls":[{"id":"call_1","type":"function","function":{"name":"act","arguments":"{\"says\":[],\"intents\":[]}"}}]}}]}"#;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                resp.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (port, posts)
}

fn read_http(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return String::from_utf8_lossy(&buf).into_owned(),
            Ok(_) => {
                buf.push(byte[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    let header = String::from_utf8_lossy(&buf);
    let mut length = 0usize;
    for line in header.lines() {
        if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = rest.trim().parse().unwrap_or(0);
        }
    }
    let header_len = buf.len();
    while buf.len() < header_len + length {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => buf.push(byte[0]),
        }
    }
    String::from_utf8_lossy(&buf[header_len..]).into_owned()
}

fn wait_posts(posts: &Mutex<Vec<String>>, count: usize) {
    let start = Instant::now();
    loop {
        if posts.lock().unwrap().len() >= count {
            return;
        }
        if start.elapsed() > Duration::from_secs(8) {
            panic!("stub saw {} posts", posts.lock().unwrap().len());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_text(slot: &Arc<Mutex<String>>, needle: &str) {
    let start = Instant::now();
    loop {
        if support::text_of(slot).contains(needle) {
            return;
        }
        if start.elapsed() > Duration::from_secs(8) {
            panic!("missing {needle}\n{}", support::text_of(slot));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn drain(reader: impl Read + Send + 'static, slot: Arc<Mutex<String>>) {
    thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => slot
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });
}
