use spookery_engine::testing::{place_of, world_of};
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use cucumber::{given, then, when};
use spookery::agent::{model, CompletionError};
use spookery::http_model::{HttpModel, Settings, MODEL_CALL_TIMEOUT, RETRY_BACKOFF};

use crate::support::{self, copy_pack};
use crate::World;

use super::host;
use super::scripted_gm::{
    read_tool_results, render_completion, start_ai_gm, stop_ai_gm, AiGm, ScriptedModel, Seen, HOLD,
};

#[when(expr = "the operator runs ai-gm with an extra argument")]
async fn the_operator_runs_ai_gm_with_an_extra_argument(world: &mut World) {
    let (listener, addr) = free_listener();
    world.listeners = vec![listener];
    host::run(world, &["ai-gm", "extra", "--addr", &addr]).await;
}

#[when(expr = "the operator runs ai-gm with nothing listening")]
fn the_operator_runs_ai_gm_with_nothing_listening(world: &mut World) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["ai-gm", "--addr", "127.0.0.1:1"]);
    command.current_dir(env!("CARGO_TARGET_TMPDIR"));
    apply_model_env(&mut command, None);
    let output = command
        .output()
        .unwrap_or_else(|err| panic!("spawn ai-gm: {err}"));
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[when(expr = "the operator runs ai-gm without {string}")]
fn the_operator_runs_ai_gm_without(world: &mut World, var: String) {
    let (listener, addr) = free_listener();
    let listeners = [listener];
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["ai-gm", "--addr", &addr]);
    command.current_dir(env!("CARGO_TARGET_TMPDIR"));
    apply_model_env(&mut command, Some(&var));
    let output = command
        .output()
        .unwrap_or_else(|err| panic!("spawn ai-gm: {err}"));
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
fn stderr_names_as_unset(world: &mut World, var: String) {
    assert!(
        world.stderr.contains(&format!("{var} is unset")),
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

#[when(expr = "the AI GM is pointed at a game port")]
async fn the_gm_is_pointed_at_a_game_port(world: &mut World) {
    let (listener, addr) = free_listener();
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command
        .args(["ai-gm", "--addr", &addr])
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .stdin(Stdio::null());
    apply_model_env(&mut command, None);
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn ai-gm: {err}"));
    world.children.push(child);
    host::poll_until(&format!("a dial on {addr}"), async || {
        match listener.accept() {
            Ok(_) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Err("no dial"),
            Err(err) => panic!("accept: {err}"),
        }
    })
    .await;
    world.scratch = "dialed".to_string();
}

#[then(expr = "the AI GM dialed that address")]
fn the_gm_dialed_that_address(world: &mut World) {
    assert_eq!(world.scratch, "dialed");
}

#[then(expr = "the player saw speech from the GM")]
fn the_player_saw_speech(world: &mut World) {
    assert!(
        world.player_added.contains("{\"kind\":\"say\""),
        "{}",
        world.player_added
    );
}

#[then(expr = "the tape from is gm")]
fn the_tape_from_is_gm(world: &mut World) {
    let tape = fs::read_to_string(host::pack(world).join("state/tape.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"));
    assert!(tape.contains("\"from\":\"gm\""), "{tape}");
}

#[when(expr = "play and the AI GM and a stub run a speech turn and a travel turn")]
async fn play_and_ai_gm_and_a_stub(world: &mut World) {
    let pack = copy_pack("docks", "host-stub");
    let (stub_port, seen) = start_model_stub(std::iter::repeat(Reply::Tools(&[])));
    host::listen(world, &pack).await;
    let addr = host::game_addr(world);
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command
        .args(["ai-gm", "--addr", &addr])
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .stdin(Stdio::null());
    apply_model_env(&mut command, None);
    let mut ai_gm = command
        .env("SPOOK_GM_URL", format!("http://127.0.0.1:{stub_port}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn ai-gm: {err}"));
    let ai_gm_stdout = Arc::new(Mutex::new(String::new()));
    let ai_gm_stderr = Arc::new(Mutex::new(String::new()));
    support::drain(ai_gm.stdout.take().unwrap(), Arc::clone(&ai_gm_stdout));
    support::drain(ai_gm.stderr.take().unwrap(), Arc::clone(&ai_gm_stderr));
    world.children.push(ai_gm);
    host::wait_suffix(&ai_gm_stderr, 0, "listening").await;
    let mut play = super::common::spawn_play(&addr, "rowan", &[]);
    host::wait_suffix(&play.stdout, 0, "Docks Shop").await;
    let mut play_in = play.stdin.take().unwrap();
    writeln!(play_in, "say hello").unwrap();
    play_in.flush().unwrap();
    wait_stub(&seen, 1, 0).await;
    assert_eq!(world_of(&pack).world_time, 1080, "speech played a round");
    let first = seen.lock().unwrap().posts[0].clone();
    assert!(first.contains("end_turn"), "{first}");
    assert_chat_post(&first);
    let mark = support::text_of(&play.stdout).len();
    writeln!(play_in, "travel docks_street").unwrap();
    play_in.flush().unwrap();
    wait_stub(&seen, 2, 0).await;
    host::wait_suffix(&play.stdout, mark, "You are in Docks Street.").await;
    assert_eq!(place_of(&world_of(&pack), "rowan"), "docks_street");
    let second = seen.lock().unwrap().posts[1].clone();
    assert!(second.contains("end_turn"), "{second}");
    assert_chat_post(&second);
    let mark = support::text_of(&play.stdout).len();
    writeln!(play_in, "look").unwrap();
    play_in.flush().unwrap();
    host::wait_suffix(&play.stdout, mark, "Exits:").await;
    assert_eq!(world_of(&pack).world_time, 1085, "travel played two rounds");
    assert_eq!(seen.lock().unwrap().posts.len(), 2);
    assert!(
        support::text_of(&ai_gm_stdout).is_empty(),
        "{}",
        support::text_of(&ai_gm_stdout)
    );
    drop(play_in);
    drop(play);
    host::stop(world);
    let _ = fs::remove_dir_all(&pack);
}

#[given(expr = "a docks table with the scripted AI GM and a seated player")]
async fn a_docks_table_with_the_scripted_ai_gm_and_a_seated_player(world: &mut World) {
    let pack = copy_pack("docks", "host-turns");
    host::listen(world, &pack).await;
    world.ai_gm = Some(start_ai_gm(world, world.model.clone()).await);
    host::seat_rowan(world, &pack).await;
    world.start_ticks = world_of(&pack).world_time;
    world.pack = Some(pack);
}

#[given(expr = "a scripted model that says evening to the table")]
fn a_scripted_model_that_says_evening(world: &mut World) {
    world
        .model
        .push(&[("say", r#"{"to":"table","text":"evening"}"#)]);
}

#[given(expr = "a scripted model whose first call fails in transport")]
fn a_scripted_model_whose_first_call_fails_in_transport(world: &mut World) {
    world.model.push_error(CompletionError::Transport(
        "model transport error".to_string(),
    ));
}

#[given(expr = "a scripted model that ends the turn, then says the roll lands")]
fn a_scripted_model_that_ends_the_turn_then_says_the_roll_lands(world: &mut World) {
    world.model.push(&[("end_turn", "{}")]);
    world
        .model
        .push(&[("say", r#"{"to":"table","text":"the roll lands"}"#)]);
}

#[when(expr = "the player says hello")]
async fn the_player_says_hello(world: &mut World) {
    let (mark, _) = host::player_send(world, "say hello", "\"kind\":\"say\"").await;
    host::wait_player_from(world, mark, "evening").await;
}

#[when(expr = "the player travels to Docks Street")]
async fn the_player_travels_to_docks_street(world: &mut World) {
    let (mark, _) = host::player_send(world, "travel docks_street", "\"kind\":\"queued\"").await;
    world.round_ticks = host::wait_rounds_to_view(world, mark).await;
}

#[when(expr = "the player tries exert")]
async fn the_player_tries_exert(world: &mut World) {
    let (mark, _) = host::player_send(world, "exert", "\"kind\":\"queued\"").await;
    world.round_ticks = host::wait_rounds_to_view(world, mark).await;
    host::wait_player_from(world, mark, "the roll lands").await;
}

#[then(expr = "world time moved one round")]
fn world_time_moved_one_round(world: &mut World) {
    let world_time = world_of(host::pack(world)).world_time;
    assert_eq!(
        world_time,
        world.start_ticks + 5,
        "the turn played another round"
    );
}

#[then(expr = "the GM's own speech did not call the model")]
async fn the_gms_own_speech_did_not_call_the_model(world: &mut World) {
    let calls = host::poll_until("a model call that shows bye", async || {
        let calls = world.model.calls();
        let bye = calls.iter().position(|call| observation_shows(call, "bye"));
        bye.map(|bye| calls[..bye].to_vec()).ok_or(calls.len())
    })
    .await;
    for call in &calls {
        assert!(!observation_shows(call, "evening"), "{call:?}");
    }
}

/// Whether a user message of the model call, its observation, contains `text`.
fn observation_shows(call: &[model::Message], text: &str) -> bool {
    call.iter().any(
        |message| matches!(message, model::Message::User { content } if content.contains(text)),
    )
}

#[then(expr = "one round committed")]
#[then(expr = "the queued line committed one round")]
fn one_round_committed(world: &mut World) {
    assert_eq!(
        world.round_ticks,
        [world.start_ticks + 5],
        "more than one round committed"
    );
}

#[then(expr = "the model narrated the rolls in the same turn")]
async fn the_model_narrated_the_rolls_in_the_same_turn(world: &mut World) {
    let calls = world.model.wait_past_script().await;
    let rolls = read_tool_results(&calls[1]);
    assert!(rolls[0].contains("\"rolls\""), "{rolls:?}");
}

#[when(expr = "a player queues travel while the model answers their say")]
async fn a_player_queues_travel_while_the_model_answers_their_say(world: &mut World) {
    let pack = copy_pack("docks", "host-close-turns");
    host::listen(world, &pack).await;
    let model = ScriptedModel::default();
    model.push(&[]);
    model.push(&[]);
    model.hold_next();
    let gm = start_ai_gm(world, model.clone()).await;
    host::seat_rowan(world, &pack).await;
    world.start_ticks = world_of(&pack).world_time;
    host::player_send(world, "say hello", "\"kind\":\"say\"").await;
    host::poll_until("the say's model call", async || match model.calls().len() {
        1 => Ok(()),
        calls => Err(calls),
    })
    .await;
    let (mark, _) = host::player_send(world, "travel docks_street", "\"kind\":\"queued\"").await;
    gm.advance(HOLD).await;
    world.round_ticks = host::wait_rounds_to_view(world, mark).await;
    stop_ai_gm(world, gm);
    world.pack = Some(pack);
}

fn tape_text(pack: &Path) -> String {
    fs::read_to_string(pack.join("state/tape.jsonl")).unwrap_or_default()
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

const STUB_SAY: &[(&str, &str)] = &[("say", r#"{"to":"table","text":"STUB-SAY-FIXTURE"}"#)];

#[when(expr = "a hung model call meets a queued player line")]
async fn a_hung_model_call_meets_a_queued_player_line(world: &mut World) {
    let (pack, _) = queued_travel(world, "host-hang").await;
    let (stub, seen) = start_model_stub([Reply::Hang, Reply::Hang]);
    let gm = start_ai_gm(world, http_model(stub)).await;
    wait_stub(&seen, 1, 0).await;
    gm.advance(MODEL_CALL_TIMEOUT).await;
    wait_stub(&seen, 1, 1).await;
    assert_eq!(
        seen.lock().unwrap().posts.len(),
        1,
        "retried before the delay"
    );
    gm.advance(RETRY_BACKOFF).await;
    wait_stub(&seen, 2, 1).await;
    gm.advance(MODEL_CALL_TIMEOUT).await;
    wait_stub(&seen, 2, 2).await;
    world.round_ticks = host::wait_rounds_to_view(world, world.mark).await;
    world.ai_gm = Some(gm);
    world.pack = Some(pack);
    world.stub_seen = Some(seen);
}

#[when(expr = "a model answers 500 once and then speaks")]
async fn a_model_answers_500_once_and_then_speaks(world: &mut World) {
    let (pack, _) = queued_travel(world, "host-retry").await;
    let replies = [
        Reply::Status(500, "{\"error\":\"busy\"}"),
        Reply::Tools(STUB_SAY),
        Reply::Tools(&[]),
    ];
    let (stub, seen) = start_model_stub(replies);
    let gm = start_ai_gm(world, http_model(stub)).await;
    wait_stub(&seen, 1, 1).await;
    gm.advance(RETRY_BACKOFF).await;
    finish_http_turn(world, gm, pack, seen).await;
}

#[when(expr = "a model answers 401")]
async fn a_model_answers_401(world: &mut World) {
    let (pack, _) = queued_travel(world, "host-unauthorized").await;
    let (stub, seen) = start_model_stub([Reply::Status(401, UNAUTHORIZED_BODY)]);
    let gm = start_ai_gm(world, http_model(stub)).await;
    host::wait_suffix(&gm.err, 0, "model call returned 401").await;
    finish_http_turn(world, gm, pack, seen).await;
}

/// A provider 401 body that echoes the key back, the way some providers do.
const UNAUTHORIZED_BODY: &str =
    "{\"error\":{\"message\":\"Incorrect API key provided: test-key\",\n \"code\":\"invalid_api_key\"}}";

#[then(expr = "the error line shows the provider's text without the key")]
fn the_error_line_shows_the_providers_text_without_the_key(world: &mut World) {
    let err = &world.stderr;
    let line = err
        .lines()
        .find(|line| line.starts_with("model call returned 401: "))
        .unwrap_or_else(|| panic!("no 401 line\n{err}"));
    assert!(line.contains("Incorrect API key provided"), "{line}");
    assert!(line.contains("invalid_api_key"), "{line}");
    assert!(!err.contains("test-key"), "{err}");
}

#[then(expr = "the hung call retried once after the delay")]
fn the_hung_call_retried_once_after_the_delay(world: &mut World) {
    assert_eq!(lock_stub_seen(world).posts.len(), 2);
}

#[then(expr = "the stub saw {int} model post")]
fn the_stub_saw_model_posts(world: &mut World, count: usize) {
    assert_eq!(lock_stub_seen(world).posts.len(), count);
}

#[then(expr = "both hung calls closed")]
fn both_hung_calls_closed(world: &mut World) {
    assert_eq!(lock_stub_seen(world).closes, 2);
}

#[then(expr = "the model's say landed")]
fn the_models_say_landed(world: &mut World) {
    let tape = tape_text(host::pack(world));
    assert!(tape.contains("STUB-SAY-FIXTURE"), "{tape}");
}

#[then(expr = "the model's say did not land")]
fn the_models_say_did_not_land(world: &mut World) {
    let tape = tape_text(host::pack(world));
    assert!(!tape.contains("STUB-SAY-FIXTURE"), "{tape}");
}

fn lock_stub_seen(world: &World) -> std::sync::MutexGuard<'_, Seen> {
    world
        .stub_seen
        .as_ref()
        .expect("a model stub")
        .lock()
        .unwrap()
}

/// Waits for the queued travel's round, then stops the AI GM and keeps its error text, the
/// pack and what the stub saw.
async fn finish_http_turn(world: &mut World, gm: AiGm, pack: PathBuf, seen: Arc<Mutex<Seen>>) {
    world.round_ticks = host::wait_rounds_to_view(world, world.mark).await;
    world.stderr = support::text_of(&gm.err);
    stop_ai_gm(world, gm);
    for post in &seen.lock().unwrap().posts {
        assert_chat_post(post);
    }
    world.pack = Some(pack);
    world.stub_seen = Some(seen);
}

/// A pack copy with Rowan seated and travel queued while a GM held the seat, then let go.
pub(crate) async fn queued_travel(world: &mut World, label: &str) -> (PathBuf, u64) {
    let pack = copy_pack("docks", label);
    host::listen(world, &pack).await;
    host::seat_gm(world).await;
    host::seat_rowan(world, &pack).await;
    world.start_ticks = world_of(&pack).world_time;
    world.mark = host::player_send(world, "travel docks_street", "\"kind\":\"queued\"")
        .await
        .0;
    assert_eq!(
        world_of(&pack).world_time,
        world.start_ticks,
        "travel committed before the host"
    );
    host::release_gm(world).await;
    (pack, world.start_ticks)
}

fn http_model(port: u16) -> HttpModel {
    HttpModel::new(Settings::new(
        "test-key",
        format!("http://127.0.0.1:{port}"),
        "grok-test",
    ))
    .unwrap_or_else(|err| panic!("{err}"))
}

/// One answer from the model stub.
#[derive(Clone, Copy)]
enum Reply {
    /// Read the request and never answer.
    Hang,
    /// Answer with this HTTP status and body.
    Status(u16, &'static str),
    /// Answer 200 with these tool calls, each a name and its arguments.
    Tools(&'static [(&'static str, &'static str)]),
}

/// A chat-completions endpoint on a free port. Each connection gets the next reply, then
/// the stub waits for the client to close it. When the replies run out it stops listening.
fn start_model_stub(
    replies: impl IntoIterator<Item = Reply, IntoIter: Send + 'static>,
) -> (u16, Arc<Mutex<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("local_addr: {err}"))
        .port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let log = Arc::clone(&seen);
    let mut replies = replies.into_iter();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let (Ok(mut stream), Some(reply)) = (stream, replies.next()) else {
                break;
            };
            log.lock().unwrap().posts.push(read_http(&mut stream));
            let answer = match reply {
                Reply::Hang => None,
                Reply::Status(status, body) => Some((status, body.to_string())),
                Reply::Tools(calls) => Some((200, render_completion(calls))),
            };
            if let Some((status, body)) = answer {
                let reason = if status == 200 { "OK" } else { "Error" };
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.shutdown(Shutdown::Write);
            }
            let _ = stream.set_read_timeout(None);
            let mut buf = [0u8; 8];
            while matches!(stream.read(&mut buf), Ok(read) if read > 0) {}
            log.lock().unwrap().closes += 1;
        }
    });
    (port, seen)
}

/// Waits until the stub has read `posts` requests and seen `closes` of their connections close.
async fn wait_stub(seen: &Mutex<Seen>, posts: usize, closes: usize) {
    host::poll_until(&format!("{posts} posts and {closes} closes"), async || {
        let seen = seen.lock().unwrap();
        if seen.posts.len() >= posts && seen.closes >= closes {
            Ok(())
        } else {
            Err(format!(
                "{} posts and {} closes",
                seen.posts.len(),
                seen.closes
            ))
        }
    })
    .await;
}

fn apply_model_env(command: &mut Command, missing: Option<&str>) {
    for (var, value) in [
        ("SPOOK_GM_KEY", "test-key"),
        ("SPOOK_GM_URL", "http://127.0.0.1:9"),
        ("SPOOK_GM_MODEL", "grok-test"),
    ] {
        if missing == Some(var) {
            command.env_remove(var);
        } else {
            command.env(var, value);
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

fn read_http(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(support::WAIT_LIMIT));
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
