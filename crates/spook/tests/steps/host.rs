use std::fs;
use std::future::Future;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

use cucumber::{then, when};
use serde_json::json;
use spookery::ai::{
    parse_tool_line, run_gm, Clock, CompleteError, CompletionRequest, Env, HttpModel, Limits,
    Model, ScriptedModel, TurnWatch, WallClock,
};
use spookery::gm_connection::GmConnection;

use crate::support::{self, temp_pack};
use crate::World;

use super::table;

#[when(expr = "the operator runs ai-gm with an extra argument")]
fn the_operator_runs_host_with_an_extra_argument(world: &mut World) {
    let ([addr, door], listeners) = bind_game_and_agent();
    world.listeners = listeners;
    table::run(
        world,
        &["ai-gm", "extra", "--addr", &addr, "--mcp-http", &door],
    );
}

#[when(expr = "the operator runs ai-gm with nothing listening")]
fn the_operator_runs_host_with_nothing_listening(world: &mut World) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["ai-gm", "--addr", "127.0.0.1:1"]);
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
    let ([addr, door], listeners) = bind_game_and_agent();
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["ai-gm", "--addr", &addr, "--mcp-http", &door]);
    apply_model_env(&mut command, Some(&name));
    let output = command
        .output()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    world.scratch = if accepted_any(&listeners) {
        "dialed".to_string()
    } else {
        "quiet".to_string()
    };
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
fn nothing_was_dialed(world: &mut World) {
    if accepted_any(&world.listeners) {
        panic!("ai-gm dialed");
    }
}

#[when(expr = "the host is pointed at an overridden agent door")]
fn the_host_is_pointed_at_an_overridden_agent_door(world: &mut World) {
    let (listener, http) = free_listener();
    // Port 1's next port is 2, which an ephemeral door is not, so ignoring
    // `--mcp-http` and dialing the next port misses this listener.
    let addr = "127.0.0.1:1".to_string();
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command
        .args(["ai-gm", "--addr", &addr, "--mcp-http", &http])
        .stdin(Stdio::null());
    apply_model_env(&mut command, None);
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    world.children.push(child);
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok(_) => {
                world.scratch = "dialed".to_string();
                return;
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("ai-gm did not dial {http}");
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
    let mark = table::player_len(world);
    let door = table::door_addr(world).parse().unwrap();
    let mut seat =
        support::block_on(GmConnection::connect_door(door)).unwrap_or_else(|err| panic!("{err}"));
    let say =
        support::block_on(seat.call("say", json!({"address": "table", "text": "good evening"})))
            .unwrap_or_else(|err| panic!("{err}"));
    let again = parse_tool_line(&say.raw).unwrap_or_else(|err| panic!("{err}"));
    assert!(!say.refused, "{}", say.raw);
    assert!(!again.refused, "{}", again.raw);
    assert_eq!(again.body, say.body);
    world.player_added = table::wait_player_from(world, mark, "{\"kind\":\"say\"");
    world.scratch = say.raw;
    let refused = support::block_on(seat.call("not_in_spec", json!({})))
        .unwrap_or_else(|err| panic!("{err}"));
    support::block_on(seat.close());
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
fn the_scripted_host_plays_the_turn_checks(world: &mut World) {
    speech_leaves_world_time_and_a_gm_say_does_not_call(world);
    travel_rounds_once(world, false);
    travel_rounds_once(world, true);
    epilogue_does_not_play_another_round(world);
    sit_commits_the_waiting_queue(world);
}

#[when(expr = "play and host and a stub run a speech turn and a travel turn")]
fn play_and_host_and_a_stub(world: &mut World) {
    let pack = temp_pack("host-stub");
    let aldric = pack.join("cast/aldric.md");
    let mut body = fs::read_to_string(&aldric).unwrap();
    body.push_str("\nCASTBODYONLY-FIXTURE\n");
    fs::write(&aldric, body).unwrap();
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    let (stub_port, posts) = stub_server();
    table::listen(world, &pack);
    let addr = table::game_addr(world);
    let door = table::door_addr(world);
    let mut host_cmd = Command::new(env!("CARGO_BIN_EXE_spook"));
    host_cmd
        .args(["ai-gm", "--addr", &addr, "--mcp-http", &door])
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
    world.children.push(host);
    wait_text(&host_stderr, "listening");
    let mut play = support::spawn(&["play", "--addr", &addr, "--as", "player:pc_rowan"]);
    wait_text(&play.stdout, "Docks Shop");
    let mut play_in = play.stdin.take().unwrap();
    writeln!(play_in, "say hello").unwrap();
    play_in.flush().unwrap();
    wait_posts(&posts, 1);
    assert_eq!(world_time(&pack), 1080, "speech played a round");
    let first = posts.lock().unwrap()[0].clone();
    assert!(first.contains("act"), "{first}");
    assert!(first.contains("CASTBODYONLY-FIXTURE"), "{first}");
    assert_chat_post(&first);
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
    assert!(second.contains("CASTBODYONLY-FIXTURE"), "{second}");
    assert_chat_post(&second);
    // The world loop answers this look after any round queued before it.
    let mark = support::text_of(&play.stdout).len();
    writeln!(play_in, "look").unwrap();
    play_in.flush().unwrap();
    table::wait_suffix(&play.stdout, mark, "Exits:");
    assert_eq!(world_time(&pack), 1085, "travel played two rounds");
    assert_eq!(posts.lock().unwrap().len(), 2);
    assert!(
        support::text_of(&host_stdout).is_empty(),
        "{}",
        support::text_of(&host_stdout)
    );
    drop(play_in);
    drop(play);
    table::stop_server(world);
    let _ = fs::remove_dir_all(&pack);
}

fn speech_leaves_world_time_and_a_gm_say_does_not_call(world: &mut World) {
    with_host(world, "host-speech", |world, pack, _port, model, watch| {
        table::seat_player(world, pack);
        let sit = world_time(pack);
        model.push(Ok(Some(
            r#"{"says":[{"address":"table","text":"evening"}],"intents":[]}"#.to_string(),
        )));
        table::player_send(world, "say hello", "\"kind\":\"say\"");
        wait_idle(watch, model, 1);
        assert_eq!(world_time(pack), sit, "speech changed world_time");
        let tape = tape_text(pack);
        assert!(tape.contains("\"from\":\"gm\""), "{tape}");
        assert!(tape.contains("evening"), "{tape}");
        // The world loop answers this look after it sent the GM its own say.
        table::player_send(world, "look", LINES);
        wait_idle(watch, model, 1);
        assert_eq!(model.calls().len(), 1, "a gm say called the model");
        assert!(!watch.busy.load(Ordering::Acquire));
        assert_eq!(world_time(pack), sit);
    });
}

fn travel_rounds_once(world: &mut World, transport_error: bool) {
    let label = if transport_error {
        "host-transport"
    } else {
        "host-travel"
    };
    with_host(world, label, |world, pack, _port, model, watch| {
        if transport_error {
            model.push(Err(CompleteError::Transport(
                "model transport error".to_string(),
            )));
        } else {
            model.push(Ok(Some(r#"{"says":[],"intents":[]}"#.to_string())));
        }
        table::seat_player(world, pack);
        let sit = world_time(pack);
        table::player_send(world, "travel docks_street", "\"kind\":\"queued\"");
        table::wait_world_time(pack, sit + 5);
        wait_idle(watch, model, 1);
        // The world loop answers this look after any second round or GM event queued before it.
        table::player_send(world, "look", LINES);
        wait_idle(watch, model, 1);
        assert_eq!(world_time(pack), sit + 5, "travel ran twice");
        assert_eq!(model.calls().len(), 1, "{}", model.calls().len());
        assert_eq!(
            table::now_of(pack)
                .actors
                .get("pc_rowan")
                .map(|actor| actor.place().to_string()),
            Some("docks_street".to_string())
        );
    });
}

fn epilogue_does_not_play_another_round(world: &mut World) {
    with_host(
        world,
        "host-epilogue",
        |world, pack, _port, model, watch| {
            model.push(Ok(Some(r#"{"says":[],"intents":[]}"#.to_string())));
            model.push(Ok(Some(
                r#"{"says":[{"address":"table","text":"the roll lands"}]}"#.to_string(),
            )));
            table::seat_player(world, pack);
            let sit = world_time(pack);
            table::player_send(world, "grit", "\"kind\":\"queued\"");
            table::wait_world_time(pack, sit + 5);
            wait_idle(watch, model, 2);
            let tape = tape_text(pack);
            assert!(tape.contains("the roll lands"), "{tape}");
            let calls = model.calls();
            assert_eq!(calls.len(), 2, "epilogue did not run");
            assert!(!calls[0].says_only);
            assert!(calls[1].says_only);
            assert!(
                calls[1].prompt.contains("\"checks\""),
                "{}",
                calls[1].prompt
            );
            // The world loop answers this look after the epilogue say and any round it caused.
            table::player_send(world, "look", LINES);
            wait_idle(watch, model, 2);
            assert_eq!(world_time(pack), sit + 5, "epilogue played a round");
            assert_eq!(model.calls().len(), 2);
        },
    );
}

fn sit_commits_the_waiting_queue(world: &mut World) {
    let pack = temp_pack("host-sit");
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_player(world, &pack);
    table::player_send(world, "say good evening", "\"kind\":\"say\"");
    table::player_send(world, "travel docks_street", "\"kind\":\"queued\"");
    assert_eq!(world_time(&pack), 1080);
    table::release_gm(world);
    let model = ScriptedModel::new();
    model.push(Ok(Some(r#"{"says":[],"intents":[]}"#.to_string())));
    let (handle, watch) = start_gm(world, &model);
    table::wait_world_time(&pack, 1085);
    wait_idle(&watch, &model, 1);
    assert_eq!(
        watch.high_water.load(Ordering::Acquire),
        watch.sit_seq.load(Ordering::Acquire),
        "sit moved the high water"
    );
    assert!(watch.sit_seq.load(Ordering::Acquire) > 0);
    // The world loop answers this look after the sit's commit and any GM event it sent.
    table::player_send(world, "look", LINES);
    wait_idle(&watch, &model, 1);
    assert_eq!(world_time(&pack), 1085);
    assert_eq!(model.calls().len(), 1);
    stop_host(world, handle);
    let _ = fs::remove_dir_all(&pack);
}

fn with_host(
    world: &mut World,
    label: &str,
    body: impl FnOnce(&mut World, &Path, u16, &ScriptedModel, &TurnWatch),
) {
    let pack = temp_pack(label);
    table::listen(world, &pack);
    let port = table::live_mut(world).port;
    let model = ScriptedModel::new();
    let (handle, watch) = start_gm(world, &model);
    body(world, &pack, port, &model, &watch);
    stop_host(world, handle);
    let _ = fs::remove_dir_all(&pack);
}

fn stop_host(world: &mut World, handle: thread::JoinHandle<Result<(), spookery::ai::Error>>) {
    table::live_mut(world).server.take();
    let _ = handle.join();
}

fn start_gm(
    world: &mut World,
    model: &ScriptedModel,
) -> (
    thread::JoinHandle<Result<(), spookery::ai::Error>>,
    Arc<TurnWatch>,
) {
    let watch = Arc::new(TurnWatch::default());
    let watch_bg = Arc::clone(&watch);
    let model_bg = model.clone();
    let addr = table::game_addr(world).parse().unwrap();
    let door = table::door_addr(world).parse().unwrap();
    let runtime = tokio::runtime::Handle::current();
    let handle = thread::spawn(move || {
        let mut model = model_bg;
        runtime.block_on(run_gm(addr, Some(door), &mut model, &watch_bg))
    });
    wait_listening(&watch);
    (handle, watch)
}

/// The reply to a player `look`, which takes no round.
const LINES: &str = "\"kind\":\"lines\"";

fn wait_listening(watch: &TurnWatch) {
    let start = Instant::now();
    while !watch.listening.load(Ordering::Acquire) {
        if start.elapsed() > Duration::from_secs(5) {
            panic!("host did not listen");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_idle(watch: &TurnWatch, model: &ScriptedModel, calls: usize) {
    let start = Instant::now();
    loop {
        let ready = model.calls().len() >= calls && !watch.busy.load(Ordering::Acquire);
        if ready {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "host still busy calls {} busy {}",
                model.calls().len(),
                watch.busy.load(Ordering::Acquire)
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn world_time(pack: &Path) -> u64 {
    let report = spookery_engine::testing::replay_journal(pack, None, None)
        .unwrap_or_else(|err| panic!("{err}"));
    let text = std::str::from_utf8(&report.now_yaml).unwrap();
    spookery_engine::load_save(text).unwrap().world_time
}

fn tape_text(pack: &Path) -> String {
    fs::read_to_string(pack.join("state/journal/session.jsonl")).unwrap_or_default()
}

/// The game port and the agent door, each a free port of its own. Callers
/// pass the door with `--mcp-http`, so no step needs two adjacent ports.
fn bind_game_and_agent() -> ([String; 2], Vec<TcpListener>) {
    let (game, game_addr) = free_listener();
    let (door, door_addr) = free_listener();
    ([game_addr, door_addr], vec![game, door])
}

fn free_listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|err| panic!("nonblocking: {err}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("local_addr: {err}"))
        .to_string();
    (listener, addr)
}

fn accepted_any(listeners: &[TcpListener]) -> bool {
    for listener in listeners {
        match listener.accept() {
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(_) => return true,
            Err(err) => panic!("accept: {err}"),
        }
    }
    false
}

const SAY_ARGUMENTS: &str =
    r#"{"says":[{"address":"table","text":"STUB-SAY-FIXTURE"}],"intents":[]}"#;

#[when(expr = "a hung model call meets a queued player line")]
fn a_hung_model_call_meets_a_queued_player_line(world: &mut World) {
    let (pack, sit) = queued_travel(world, "host-hang");
    let total = Duration::from_secs(1);
    let backoff = Duration::from_secs(2);
    let clock = TestClock::new();
    let (stub, events) = hang_server(clock.clone(), total, backoff);
    let (handle, watch) = drive_when_free(world, stub, total, backoff, clock);
    let advanced = wait_time(&pack, sit + 5, Duration::from_secs(8));
    let log = events.lock().unwrap().clone();
    let up = watch.listening.load(Ordering::Acquire) && !handle.is_finished();
    stop_host(world, handle);
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
    world.scratch = hung_scratch(&log, total, backoff);
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
    let mut model = http_model(
        stub,
        Duration::from_secs(5),
        Duration::from_millis(50),
        WallClock,
    );
    let request = CompletionRequest {
        prompt: "the table is quiet".to_string(),
        says_only: false,
    };
    match support::block_on(model.complete(&request)) {
        Ok(reply) => panic!("expected an error, got {reply:?}"),
        Err(err) => err.to_string(),
    }
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

#[then(expr = "the queued line committed one round")]
fn the_queued_line_committed_one_round(world: &mut World) {
    assert_eq!(world.stdout, "advanced", "{}", world.scratch);
}

#[then(expr = "the hung call retried once after the delay")]
fn the_hung_call_retried_once_after_the_delay(world: &mut World) {
    assert_eq!(hung_field(&world.scratch, "opens"), 2, "{}", world.scratch);
    let gap = hung_field(&world.scratch, "gap_ms");
    let timeout = hung_field(&world.scratch, "timeout_ms");
    let delay = hung_field(&world.scratch, "delay_ms");
    assert_eq!(gap, timeout + delay, "{}", world.scratch);
}

#[then(expr = "both hung calls closed")]
fn both_hung_calls_closed(world: &mut World) {
    assert_eq!(hung_field(&world.scratch, "closes"), 2, "{}", world.scratch);
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
    let (pack, sit) = queued_travel(world, label);
    let (stub, posts) = scripted_stub(replies);
    let (handle, _watch) = drive_when_free(
        world,
        stub,
        Duration::from_secs(5),
        Duration::from_millis(50),
        WallClock,
    );
    let advanced = wait_time(&pack, sit + 5, Duration::from_secs(8));
    let tape = tape_text(&pack);
    let recorded = posts.lock().unwrap().clone();
    for post in &recorded {
        assert_chat_post(post);
    }
    stop_host(world, handle);
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

fn queued_travel(world: &mut World, label: &str) -> (std::path::PathBuf, u64) {
    let pack = temp_pack(label);
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_player(world, &pack);
    let sit = world_time(&pack);
    table::player_send(world, "travel docks_street", "\"kind\":\"queued\"");
    assert_eq!(world_time(&pack), sit, "travel committed before the host");
    table::release_gm(world);
    (pack, sit)
}

fn drive_when_free<C>(
    world: &World,
    stub: u16,
    total: Duration,
    backoff: Duration,
    clock: C,
) -> (
    thread::JoinHandle<Result<(), spookery::ai::Error>>,
    Arc<TurnWatch>,
)
where
    C: Clock + Clone + 'static,
{
    let mut last = String::new();
    for _ in 0..40 {
        let (handle, watch) = spawn_model(world, stub, total, backoff, clock.clone());
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

fn http_model<C: Clock>(port: u16, total: Duration, backoff: Duration, clock: C) -> HttpModel<C> {
    HttpModel::with_limits(
        Env::new("test-key", format!("http://127.0.0.1:{port}"), "grok-test"),
        Limits {
            total,
            connect: Duration::from_secs(1),
            backoff,
        },
        clock,
    )
    .unwrap_or_else(|err| panic!("{err}"))
}

fn spawn_model<C>(
    world: &World,
    stub: u16,
    total: Duration,
    backoff: Duration,
    clock: C,
) -> (
    thread::JoinHandle<Result<(), spookery::ai::Error>>,
    Arc<TurnWatch>,
)
where
    C: Clock + 'static,
{
    let watch = Arc::new(TurnWatch::default());
    let watch_bg = Arc::clone(&watch);
    let addr = table::game_addr(world).parse().unwrap();
    let door = table::door_addr(world).parse().unwrap();
    let runtime = tokio::runtime::Handle::current();
    let handle = thread::spawn(move || {
        let mut model = http_model(stub, total, backoff, clock);
        runtime.block_on(run_gm(addr, Some(door), &mut model, &watch_bg))
    });
    (handle, watch)
}

fn wait_time(pack: &Path, expected: u64, bound: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < bound {
        if let Ok(report) = spookery_engine::testing::replay_journal(pack, None, None) {
            if let Ok(text) = std::str::from_utf8(&report.now_yaml) {
                if let Ok(now) = spookery_engine::load_save(text) {
                    if now.world_time == expected {
                        return true;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Logical clock for the hung-call scenario. `wait` completes when `release`
/// has granted the duration. `abandon` completes when the stub has seen the socket close.
#[derive(Clone)]
struct TestClock {
    inner: Arc<Mutex<TestClockState>>,
}

struct TestClockState {
    now: Duration,
    credit: Duration,
    closes: usize,
    seen_closes: usize,
    wakers: [Option<Waker>; 2],
}

#[derive(Clone, Copy)]
enum Waiter {
    Credit = 0,
    Close = 1,
}

impl TestClockState {
    fn waker(&mut self, waiter: Waiter) -> &mut Option<Waker> {
        &mut self.wakers[waiter as usize]
    }
}

fn wake(waker: Option<Waker>) {
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Check, park, then check again, so a grant that lands during the park is not lost.
fn poll_grant(
    done: &mut bool,
    state: &mut TestClockState,
    waiter: Waiter,
    granted: impl Fn(&mut TestClockState) -> bool,
    waker: &Waker,
) -> Poll<()> {
    if *done {
        return Poll::Ready(());
    }
    if granted(state) {
        *done = true;
        return Poll::Ready(());
    }
    *state.waker(waiter) = Some(waker.clone());
    if granted(state) {
        *done = true;
        *state.waker(waiter) = None;
        return Poll::Ready(());
    }
    Poll::Pending
}

impl TestClock {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TestClockState {
                now: Duration::ZERO,
                credit: Duration::ZERO,
                closes: 0,
                seen_closes: 0,
                wakers: [None, None],
            })),
        }
    }

    fn now(&self) -> Duration {
        self.inner.lock().unwrap().now
    }

    fn release(&self, duration: Duration) {
        let waker = {
            let mut state = self.inner.lock().unwrap();
            state.now += duration;
            state.credit += duration;
            state.waker(Waiter::Credit).take()
        };
        wake(waker);
    }

    fn note_close(&self) {
        let waker = {
            let mut state = self.inner.lock().unwrap();
            state.closes += 1;
            state.waker(Waiter::Close).take()
        };
        wake(waker);
    }
}

impl Clock for TestClock {
    fn wait(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        let clock = self.clone();
        let mut done = false;
        std::future::poll_fn(move |cx| {
            let mut state = clock.inner.lock().unwrap();
            poll_grant(
                &mut done,
                &mut state,
                Waiter::Credit,
                |state| {
                    if state.credit < duration {
                        return false;
                    }
                    state.credit -= duration;
                    true
                },
                cx.waker(),
            )
        })
    }

    fn abandon(&self) -> impl Future<Output = ()> + Send {
        let clock = self.clone();
        let mut done = false;
        std::future::poll_fn(move |cx| {
            let mut state = clock.inner.lock().unwrap();
            poll_grant(
                &mut done,
                &mut state,
                Waiter::Close,
                |state| {
                    if state.closes <= state.seen_closes {
                        return false;
                    }
                    state.seen_closes += 1;
                    true
                },
                cx.waker(),
            )
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum HangEvent {
    Opened(Duration),
    Closed,
}

fn hung_scratch(log: &[HangEvent], timeout: Duration, backoff: Duration) -> String {
    let opens: Vec<Duration> = log
        .iter()
        .filter_map(|event| match event {
            HangEvent::Opened(at) => Some(*at),
            HangEvent::Closed => None,
        })
        .collect();
    let gap = match opens.as_slice() {
        [first, second, ..] => second.saturating_sub(*first),
        _ => Duration::ZERO,
    };
    let closes = log
        .iter()
        .filter(|event| matches!(event, HangEvent::Closed))
        .count();
    format!(
        "opens={} gap_ms={} timeout_ms={} delay_ms={} closes={} log={log:?}",
        opens.len(),
        gap.as_millis(),
        timeout.as_millis(),
        backoff.as_millis(),
        closes
    )
}

fn hung_field(scratch: &str, key: &str) -> u128 {
    scratch
        .split_whitespace()
        .find_map(|part| {
            let (name, value) = part.split_once('=')?;
            if name == key {
                value.parse().ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("missing {key} in {scratch}"))
}

fn hang_server(
    clock: TestClock,
    timeout: Duration,
    backoff: Duration,
) -> (u16, Arc<Mutex<Vec<HangEvent>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("local_addr: {err}"))
        .port();
    let events = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&events);
    thread::spawn(move || {
        let mut seen = 0usize;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            log.lock().unwrap().push(HangEvent::Opened(clock.now()));
            // The client is blocked in `wait(timeout)` until this grant.
            clock.release(timeout);
            let _ = read_http(&mut stream);
            let _ = stream.set_read_timeout(None);
            let mut buf = [0u8; 8];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            log.lock().unwrap().push(HangEvent::Closed);
            clock.note_close();
            seen += 1;
            if seen == 1 {
                clock.release(backoff);
            }
        }
    });
    (port, events)
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
