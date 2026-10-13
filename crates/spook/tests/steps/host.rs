use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cucumber::{then, when};
use serde_json::json;
use spookery::ai::{
    parse_tool_line, run_ai_gm, CompleteError, CompletionRequest, Env, HttpModel, Limits, Model,
    Script, Watch,
};
use spookery::gm_connection::GmConnection;

use crate::support::{self, pack_arg, temp_pack};
use crate::World;

use super::table;

#[when(expr = "the operator runs ai-gm with no pack")]
fn the_operator_runs_host_with_no_pack(world: &mut World) {
    let addr = arm_listener();
    table::run(world, &["ai-gm", "--addr", &addr]);
}

#[when(expr = "the operator runs ai-gm with an extra argument")]
fn the_operator_runs_host_with_an_extra_argument(world: &mut World) {
    let addr = arm_listener();
    let pack = pack_arg(table::pack(world));
    table::run(world, &["ai-gm", &pack, "extra", "--addr", &addr]);
}

#[when(expr = "the operator runs ai-gm with nothing listening")]
fn the_operator_runs_host_with_nothing_listening(world: &mut World) {
    let port = table::free_port();
    let addr = format!("127.0.0.1:{port}");
    let pack = pack_arg(table::pack(world));
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["ai-gm", &pack, "--addr", &addr]);
    apply_model_env(&mut command, None);
    let output = command
        .output()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[when(expr = "the operator runs ai-gm without {string}")]
fn the_operator_runs_host_without(world: &mut World, name: String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|err| panic!("nonblocking: {err}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("local_addr: {err}"))
        .to_string();
    let pack = pack_arg(table::pack(world));
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["ai-gm", &pack, "--addr", &addr]);
    apply_model_env(&mut command, Some(&name));
    let output = command
        .output()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    match listener.accept() {
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
            world.scratch = "quiet".to_string();
        }
        other => panic!("listener accepted a connection: {other:?}"),
    }
}

#[then(expr = "stderr names {string} as unset")]
fn stderr_names_as_unset(world: &mut World, name: String) {
    assert!(
        world.stderr.contains(&format!("{name} is unset")),
        "{}",
        world.stderr
    );
}

#[then(expr = "the listener accepted no connection")]
fn the_listener_accepted_no_connection(world: &mut World) {
    assert_eq!(world.scratch, "quiet", "{}", world.stderr);
}

#[then(expr = "nothing was dialed")]
fn nothing_was_dialed(_world: &mut World) {
    let listener = LISTENER
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
        .unwrap_or_else(|| panic!("no listener"));
    match listener.accept() {
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("ai-gm dialed: {other:?}"),
    }
}

#[when(expr = "the host is pointed at a free loopback address")]
fn the_host_is_pointed_at_a_free_loopback_address(world: &mut World) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|err| panic!("nonblocking: {err}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("local_addr: {err}"))
        .to_string();
    let pack = pack_arg(table::pack(world));
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command
        .args(["ai-gm", &pack, "--addr", &addr])
        .stdin(Stdio::null());
    apply_model_env(&mut command, None);
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    crate::track_child(child);
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok(_) => {
                world.scratch = "dialed".to_string();
                return;
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("ai-gm did not dial {addr}");
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("accept: {err}"),
        }
    }
}

#[then(expr = "the host dialed that address")]
fn the_host_dialed_that_address(world: &mut World) {
    assert_eq!(world.scratch, "dialed");
}

#[when(expr = "the host seat says good evening and calls an unknown tool")]
fn the_host_seat_says(world: &mut World) {
    let port = crate::with_live(|live| live.port);
    let mark = table::player_len();
    let mut seat =
        GmConnection::connect(&format!("127.0.0.1:{port}")).unwrap_or_else(|err| panic!("{err}"));
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
    let tape = fs::read_to_string(table::pack(world).join("state/journal/session.jsonl"))
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
    let pack = temp_pack("host-stub");
    let aldric = pack.join("cast/aldric.md");
    let mut body = fs::read_to_string(&aldric).unwrap();
    body.push_str("\nCASTBODYONLY-FIXTURE\n");
    fs::write(&aldric, body).unwrap();
    spookery_kernel::commit::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    let (stub_port, posts) = stub_server();
    let pack_arg = pack_arg(&pack);
    let port = table::free_port();
    let addr = format!("127.0.0.1:{port}");
    let mut server = support::spawn(&["run", &pack_arg, "--addr", &addr]);
    wait_port(port, &mut server);
    let mut host_cmd = Command::new(env!("CARGO_BIN_EXE_spook"));
    host_cmd
        .args(["ai-gm", &pack_arg, "--addr", &addr])
        .stdin(Stdio::null());
    apply_model_env(&mut host_cmd, None);
    let mut host = host_cmd
        .env("SPOOK_GM_URL", format!("http://127.0.0.1:{stub_port}"))
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
    let mut play = support::spawn(&["play", &pack_arg, "--addr", &addr]);
    wait_text(&play.stdout, "Docks shop");
    let mut play_in = play.stdin.take().unwrap();
    writeln!(play_in, "say hello").unwrap();
    play_in.flush().unwrap();
    wait_posts(&posts, 1);
    assert_eq!(world_time(&pack), 1080, "speech ticked");
    let first = posts.lock().unwrap()[0].clone();
    assert!(first.contains("act"), "{first}");
    assert!(!first.contains("CASTBODYONLY-FIXTURE"), "{first}");
    assert_chat_post(&first);
    writeln!(play_in, "travel docks_street").unwrap();
    play_in.flush().unwrap();
    wait_posts(&posts, 2);
    table::wait_world_time(&pack, 1085);
    // The server still holds this pack. World::open runs recover and can collide with its commit.
    let text = fs::read_to_string(pack.join("state/save.yaml")).unwrap();
    let place = spookery_kernel::store::codec::load_now(&text)
        .unwrap()
        .actors
        .get("pc_rowan")
        .map(|actor| actor.place().to_string());
    assert_eq!(place.as_deref(), Some("docks_street"));
    let second = posts.lock().unwrap()[1].clone();
    assert!(second.contains("act"), "{second}");
    assert!(!second.contains("CASTBODYONLY-FIXTURE"), "{second}");
    assert_chat_post(&second);
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
        run_ai_gm(&pack_bg, &addr, &mut script, &watch_bg)
    });
    wait_listening(&watch);
    body(&pack, port, &script, &watch);
    stop_host(handle);
    let _ = fs::remove_dir_all(&pack);
}

fn stop_host(handle: thread::JoinHandle<Result<(), spookery::ai::HostError>>) {
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
    thread::JoinHandle<Result<(), spookery::ai::HostError>>,
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
            run_ai_gm(&pack_bg, &addr, &mut script, &watch_bg)
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
    let text = fs::read_to_string(pack.join("state/save.yaml")).unwrap();
    spookery_kernel::store::codec::load_now(&text)
        .unwrap()
        .world_time
}

fn tape_text(pack: &Path) -> String {
    fs::read_to_string(pack.join("state/journal/session.jsonl")).unwrap_or_default()
}

static LISTENER: Mutex<Option<TcpListener>> = Mutex::new(None);

fn arm_listener() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|err| panic!("nonblocking: {err}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("local_addr: {err}"))
        .to_string();
    *LISTENER.lock().unwrap_or_else(|err| err.into_inner()) = Some(listener);
    addr
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
            panic!("listener did not open");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

const SAY_ARGUMENTS: &str =
    r#"{"says":[{"address":"table","text":"STUB-SAY-FIXTURE"}],"intents":[]}"#;

#[when(expr = "a hung model call meets a queued player line")]
fn a_hung_model_call_meets_a_queued_player_line(world: &mut World) {
    let (pack, port, sit) = queued_travel("host-hang");
    let (stub, hits) = hang_server();
    let (handle, watch) = drive_when_free(
        &pack,
        port,
        stub,
        Duration::from_secs(1),
        Duration::from_secs(2),
    );
    let started = Instant::now();
    let advanced = wait_time(&pack, sit + 5, Duration::from_secs(8));
    let elapsed = started.elapsed();
    let up = watch.listening.load(Ordering::Acquire) && !handle.is_finished();
    let connections = hits.load(Ordering::Acquire);
    stop_host(handle);
    let _ = fs::remove_dir_all(&pack);
    world.stdout = if advanced {
        "advanced".to_string()
    } else {
        "stuck".to_string()
    };
    world.stderr = if up {
        "up".to_string()
    } else {
        "down".to_string()
    };
    world.mark = elapsed.as_millis() as usize;
    world.scratch = connections.to_string();
}

#[when(expr = "a model answers 500 once and then a say")]
fn a_model_answers_500_once_and_then_a_say(world: &mut World) {
    status_turn(
        world,
        "host-retry",
        vec![
            (500, "{\"error\":\"busy\"}".to_string()),
            (200, act_reply(SAY_ARGUMENTS)),
        ],
    );
}

#[when(expr = "a model answers 401")]
fn a_model_answers_401(world: &mut World) {
    status_turn(
        world,
        "host-unauthorized",
        vec![(401, UNAUTHORIZED_BODY.to_string())],
    );
    world.stderr = direct_error(vec![(401, UNAUTHORIZED_BODY.to_string())]);
}

/// A provider 401 body that echoes the key back, the way some providers do.
const UNAUTHORIZED_BODY: &str =
    "{\"error\":{\"message\":\"Incorrect API key provided: test-key\",\n \"code\":\"invalid_api_key\"}}";

/// One call straight through the HTTP model against a scripted stub, returning
/// the error line the host would print.
fn direct_error(replies: Vec<(u16, String)>) -> String {
    let (stub, _posts) = scripted_stub(replies);
    thread::spawn(move || {
        let mut model = http_model(stub, Duration::from_secs(5), Duration::from_millis(50));
        let request = CompletionRequest {
            prompt: "the table is quiet".to_string(),
            says_only: false,
        };
        match model.complete(&request) {
            Ok(reply) => panic!("expected an error, got {reply:?}"),
            Err(err) => err.to_string(),
        }
    })
    .join()
    .unwrap_or_else(|_| panic!("direct call panicked"))
}

#[then(expr = "the error line shows the provider's text without the key")]
fn the_error_line_shows_the_providers_text_without_the_key(world: &mut World) {
    let line = &world.stderr;
    assert!(line.starts_with("model call returned 401: "), "{line}");
    assert!(line.contains("Incorrect API key provided"), "{line}");
    assert!(line.contains("invalid_api_key"), "{line}");
    assert!(!line.contains("test-key"), "{line}");
    assert!(!line.contains('\n'), "{line}");
}

#[then(expr = "the queued line committed one tick")]
fn the_queued_line_committed_one_tick(world: &mut World) {
    assert_eq!(world.stdout, "advanced", "{}", world.scratch);
}

#[then(expr = "the hung call finished within one retry")]
fn the_hung_call_finished_within_one_retry(world: &mut World) {
    assert_eq!(world.scratch, "2", "connections");
    assert!(
        world.mark > 3_000 && world.mark < 5_000,
        "elapsed {} ms",
        world.mark
    );
}

#[then(expr = "the seat stayed up")]
fn the_seat_stayed_up(world: &mut World) {
    assert_eq!(world.stderr, "up");
}

#[then(expr = "the stub saw {int} model posts")]
#[then(expr = "the stub saw {int} model post")]
fn the_stub_saw_model_posts(world: &mut World, count: usize) {
    assert_eq!(world.mark, count, "{}", world.scratch);
}

#[then(expr = "the model's say landed")]
fn the_models_say_landed(world: &mut World) {
    assert_eq!(world.scratch, "said");
}

#[then(expr = "the model's say did not land")]
fn the_models_say_did_not_land(world: &mut World) {
    assert_eq!(world.scratch, "silent");
}

fn status_turn(world: &mut World, label: &str, replies: Vec<(u16, String)>) {
    let (pack, port, sit) = queued_travel(label);
    let (stub, posts) = scripted_stub(replies);
    let (handle, _watch) = drive_when_free(
        &pack,
        port,
        stub,
        Duration::from_secs(5),
        Duration::from_millis(50),
    );
    let advanced = wait_time(&pack, sit + 5, Duration::from_secs(8));
    let tape = tape_text(&pack);
    let recorded = posts.lock().unwrap().clone();
    for post in &recorded {
        assert_chat_post(post);
    }
    stop_host(handle);
    let _ = fs::remove_dir_all(&pack);
    world.stdout = if advanced {
        "advanced".to_string()
    } else {
        "stuck".to_string()
    };
    world.mark = recorded.len();
    world.scratch = if tape.contains("STUB-SAY-FIXTURE") {
        "said".to_string()
    } else {
        "silent".to_string()
    };
}

fn queued_travel(label: &str) -> (std::path::PathBuf, u16, u64) {
    let pack = temp_pack(label);
    let port = table::free_port();
    let bind = format!("127.0.0.1:{port}");
    table::listen(&pack, &bind, port);
    table::seat_gm();
    table::seat_player(&pack);
    let sit = world_time(&pack);
    table::player_send("travel docks_street", "\"kind\":\"queued\"");
    assert_eq!(world_time(&pack), sit, "travel committed before the host");
    crate::with_live(|live| {
        if let Some(stream) = live.gm_write.as_mut() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        live.gm_write.take();
    });
    (pack, port, sit)
}

fn drive_when_free(
    pack: &Path,
    port: u16,
    stub: u16,
    total: Duration,
    backoff: Duration,
) -> (
    thread::JoinHandle<Result<(), spookery::ai::HostError>>,
    Arc<Watch>,
) {
    let mut last = String::new();
    for _ in 0..40 {
        let (handle, watch) = spawn_model(pack, port, stub, total, backoff);
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
        if !handle.is_finished() {
            let start = Instant::now();
            while !handle.is_finished() && start.elapsed() < Duration::from_secs(2) {
                if watch.listening.load(Ordering::Acquire) {
                    return (handle, watch);
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        if watch.listening.load(Ordering::Acquire) {
            return (handle, watch);
        }
        if !handle.is_finished() {
            return (handle, watch);
        }
        match handle.join() {
            Ok(Err(err)) => last = err.message,
            Ok(Ok(())) => last = "host exited".to_string(),
            Err(_) => last = "host thread panicked".to_string(),
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("host did not listen: {last}");
}

fn http_model(port: u16, total: Duration, backoff: Duration) -> HttpModel {
    HttpModel::with_limits(
        Env::new("test-key", format!("http://127.0.0.1:{port}"), "grok-test"),
        Limits {
            total,
            connect: Duration::from_secs(1),
            backoff,
        },
    )
    .unwrap_or_else(|err| panic!("{err}"))
}

fn spawn_model(
    pack: &Path,
    port: u16,
    stub: u16,
    total: Duration,
    backoff: Duration,
) -> (
    thread::JoinHandle<Result<(), spookery::ai::HostError>>,
    Arc<Watch>,
) {
    let watch = Arc::new(Watch::default());
    let watch_bg = Arc::clone(&watch);
    let pack_bg = pack.to_path_buf();
    let addr = format!("127.0.0.1:{port}");
    // The blocking client owns a runtime. Build and drop it off the cucumber thread.
    let handle = thread::spawn(move || {
        let mut model = http_model(stub, total, backoff);
        run_ai_gm(&pack_bg, &addr, &mut model, &watch_bg)
    });
    (handle, watch)
}

fn wait_time(pack: &Path, expected: u64, bound: Duration) -> bool {
    let start = Instant::now();
    let path = pack.join("state/save.yaml");
    while start.elapsed() < bound {
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(now) = spookery_kernel::store::codec::load_now(&text) {
                if now.world_time == expected {
                    return true;
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

fn hang_server() -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("local_addr: {err}"))
        .port();
    let hits = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&hits);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            count.fetch_add(1, Ordering::SeqCst);
            let _ = read_http(&mut stream);
            let _ = stream.set_read_timeout(None);
            let mut buf = [0u8; 8];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
    });
    (port, hits)
}

fn scripted_stub(replies: Vec<(u16, String)>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("local_addr: {err}"))
        .port();
    let posts = Arc::new(Mutex::new(Vec::new()));
    let slot = Arc::clone(&posts);
    thread::spawn(move || {
        let mut replies = replies;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            let body = read_http(&mut stream);
            slot.lock().unwrap().push(body);
            let (status, resp) = if replies.is_empty() {
                (500, "{}".to_string())
            } else {
                replies.remove(0)
            };
            let reason = if status == 200 { "OK" } else { "Error" };
            let header = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                resp.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (port, posts)
}

fn act_reply(arguments: &str) -> String {
    let arguments =
        serde_json::to_string(arguments).unwrap_or_else(|err| panic!("arguments: {err}"));
    format!(
        "{{\"id\":\"stub\",\"object\":\"chat.completion\",\"choices\":[{{\"index\":0,\"message\":{{\"role\":\"assistant\",\"content\":\"prose\",\"tool_calls\":[{{\"id\":\"call_1\",\"type\":\"function\",\"function\":{{\"name\":\"act\",\"arguments\":{arguments}}}}}]}}}}]}}"
    )
}

fn apply_model_env(command: &mut Command, missing: Option<&str>) {
    for (name, value) in [
        ("SPOOK_GM_KEY", "test-key"),
        ("SPOOK_GM_URL", "http://127.0.0.1:9"),
        ("SPOOK_GM_MODEL", "grok-test"),
    ] {
        if missing == Some(name) {
            command.env_remove(name);
        } else {
            command.env(name, value);
        }
    }
}

fn assert_chat_post(raw: &str) {
    let head = raw.split("\r\n\r\n").next().unwrap_or(raw);
    assert!(head.starts_with("POST /chat/completions "), "{head}");
    assert!(
        head.to_ascii_lowercase()
            .contains("authorization: bearer test-key"),
        "{head}"
    );
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
    String::from_utf8_lossy(&buf).into_owned()
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
