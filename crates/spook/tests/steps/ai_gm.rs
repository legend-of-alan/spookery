use spookery_engine::testing::state_of;
use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::mcp_client::GmClient;
use cucumber::{then, when};
use serde_json::json;
use spookery::agent::{parse_completion, CompletionError, CompletionRequest, Model, ModelReply};
use spookery::ai_gm;
use spookery::http_model::{HttpModel, Settings, MODEL_CALL_TIMEOUT, RETRY_BACKOFF};
use spookery::protocol::{SayArgs, ServerBody};

use crate::support::{self, copy_docks};
use crate::World;

use super::table;

#[when(expr = "the operator runs ai-gm with an extra argument")]
fn the_operator_runs_ai_gm_with_an_extra_argument(world: &mut World) {
    let (listener, addr) = free_listener();
    world.listeners = vec![listener];
    table::run(world, &["ai-gm", "extra", "--addr", &addr]);
}

#[when(expr = "the operator runs ai-gm with nothing listening")]
fn the_operator_runs_ai_gm_with_nothing_listening(world: &mut World) {
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
fn the_operator_runs_ai_gm_without(world: &mut World, name: String) {
    let (listener, addr) = free_listener();
    let listeners = [listener];
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["ai-gm", "--addr", &addr]);
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

#[when(expr = "the AI GM is pointed at a game port")]
fn ai_gm_is_pointed_at_a_game_port(world: &mut World) {
    let (listener, addr) = free_listener();
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command
        .args(["ai-gm", "--addr", &addr])
        .stdin(Stdio::null());
    apply_model_env(&mut command, None);
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn host: {err}"));
    world.children.push(child);
    table::poll_until(&format!("a dial on {addr}"), || match listener.accept() {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Err("no dial"),
        Err(err) => panic!("accept: {err}"),
    });
    world.scratch = "dialed".to_string();
}

#[then(expr = "the AI GM dialed that address")]
fn ai_gm_dialed_that_address(world: &mut World) {
    assert_eq!(world.scratch, "dialed");
}

#[when(expr = "the GM seat says good evening and calls an unknown tool")]
fn the_gm_seat_says(world: &mut World) {
    let mark = table::player_mark(world);
    let mcp_addr = table::mcp_addr(world).parse().unwrap();
    let mut gm =
        support::block_on(GmClient::connect(mcp_addr)).unwrap_or_else(|err| panic!("{err}"));
    let evening = SayArgs::Table {
        text: "good evening".to_string(),
    };
    let said =
        support::block_on(gm.call("say", &json!(evening))).unwrap_or_else(|err| panic!("{err}"));
    world.player_added = table::wait_player_from(world, mark, "{\"kind\":\"say\"");
    world.scratch = json!(said).to_string();
    let reply =
        support::block_on(gm.call("not_in_spec", &json!({}))).unwrap_or_else(|err| panic!("{err}"));
    support::block_on(gm.close());
    world.stdout = match reply.last().map(|reply| &reply.body) {
        Some(ServerBody::Refused(_)) => "refused".to_string(),
        other => format!("{other:?}"),
    };
}

#[then(expr = "the player saw a say from the GM")]
fn the_player_saw_a_say(world: &mut World) {
    assert!(
        world.player_added.contains("{\"kind\":\"say\""),
        "{}",
        world.player_added
    );
}

#[then(expr = "the tape from is gm")]
fn the_tape_from_is_gm(world: &mut World) {
    let tape = fs::read_to_string(table::pack(world).join("state/tape.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"));
    assert!(tape.contains("\"from\":\"gm\""), "{tape}");
}

#[then(expr = "the GM call was refused")]
fn the_gm_call_was_refused(world: &mut World) {
    assert_eq!(world.stdout, "refused", "{}", world.scratch);
}

#[when(expr = "the scripted AI GM plays the turn checks")]
fn the_scripted_ai_gm_plays_the_turn_checks(world: &mut World) {
    speech_leaves_world_time_and_a_gm_say_does_not_call(world);
    travel_rounds_once(world, false);
    travel_rounds_once(world, true);
    epilogue_does_not_play_another_round(world);
    sit_commits_the_waiting_queue(world);
}

#[when(expr = "play and the AI GM and a stub run a speech turn and a travel turn")]
fn play_and_ai_gm_and_a_stub(world: &mut World) {
    let pack = copy_docks("host-stub");
    let aldric = pack.join("cast/aldric.md");
    let mut body = fs::read_to_string(&aldric).unwrap();
    body.push_str("\nCASTBODYONLY-FIXTURE\n");
    fs::write(&aldric, body).unwrap();
    spookery_engine::migrate_pack(&pack).unwrap_or_else(|err| panic!("migrate: {err}"));
    let (stub_port, seen) = start_model_stub(std::iter::repeat(Reply::Act(EMPTY_ACT)));
    table::listen(world, &pack);
    let addr = table::game_addr(world);
    let mut host_cmd = Command::new(env!("CARGO_BIN_EXE_spook"));
    host_cmd
        .args(["ai-gm", "--addr", &addr])
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
    support::drain(host_out, Arc::clone(&host_stdout));
    support::drain(host_err, Arc::clone(&host_stderr));
    world.children.push(host);
    table::wait_suffix(&host_stderr, 0, "listening");
    let mut play = super::common::spawn_play(&addr, "pc_rowan", &[]);
    table::wait_suffix(&play.stdout, 0, "Docks Shop");
    let mut play_in = play.stdin.take().unwrap();
    writeln!(play_in, "say hello").unwrap();
    play_in.flush().unwrap();
    wait_stub(&seen, 1, 0);
    assert_eq!(state_of(&pack).world_time, 1080, "speech played a round");
    let first = seen.lock().unwrap().posts[0].clone();
    assert!(first.contains("act"), "{first}");
    assert!(first.contains("CASTBODYONLY-FIXTURE"), "{first}");
    assert_chat_post(&first);
    let mark = support::text_of(&play.stdout).len();
    writeln!(play_in, "travel docks_street").unwrap();
    play_in.flush().unwrap();
    wait_stub(&seen, 2, 0);
    // The view after the round shows the street.
    table::wait_suffix(&play.stdout, mark, "You are in Docks Street.");
    let place = state_of(&pack)
        .actors
        .get("pc_rowan")
        .map(|actor| actor.place.clone());
    assert_eq!(place.as_deref(), Some("docks_street"));
    let second = seen.lock().unwrap().posts[1].clone();
    assert!(second.contains("act"), "{second}");
    assert!(second.contains("CASTBODYONLY-FIXTURE"), "{second}");
    assert_chat_post(&second);
    // The table answers in order, so this look's view follows any round it played first.
    let mark = support::text_of(&play.stdout).len();
    writeln!(play_in, "look").unwrap();
    play_in.flush().unwrap();
    table::wait_suffix(&play.stdout, mark, "Exits:");
    assert_eq!(state_of(&pack).world_time, 1085, "travel played two rounds");
    assert_eq!(seen.lock().unwrap().posts.len(), 2);
    assert!(
        support::text_of(&host_stdout).is_empty(),
        "{}",
        support::text_of(&host_stdout)
    );
    drop(play_in);
    drop(play);
    table::stop_host(world);
    let _ = fs::remove_dir_all(&pack);
}

fn speech_leaves_world_time_and_a_gm_say_does_not_call(world: &mut World) {
    run_ai_gm(world, "host-speech", |world, pack, model| {
        table::seat_rowan(world, pack);
        let sit = state_of(pack).world_time;
        model.push(Ok(
            r#"{"says":[{"address":"table","text":"evening"}],"intents":[]}"#,
        ));
        let (mark, _) = table::player_send(world, "say hello", "\"kind\":\"say\"");
        table::wait_player_from(world, mark, "evening");
        assert_eq!(state_of(pack).world_time, sit, "speech changed world_time");
        let tape = tape_text(pack);
        assert!(tape.contains("\"from\":\"gm\""), "{tape}");
        assert!(tape.contains("evening"), "{tape}");
        assert_eq!(model.calls().len(), 1, "a gm say called the model");
        assert_eq!(state_of(pack).world_time, sit);
    });
}

fn travel_rounds_once(world: &mut World, transport_error: bool) {
    let label = if transport_error {
        "host-transport"
    } else {
        "host-travel"
    };
    run_ai_gm(world, label, |world, pack, model| {
        if transport_error {
            model.push(Err(CompletionError::Transport(
                "model transport error".to_string(),
            )));
        } else {
            model.push(Ok(EMPTY_ACT));
        }
        table::seat_rowan(world, pack);
        let sit = state_of(pack).world_time;
        let (mark, _) = table::player_send(world, "travel docks_street", "\"kind\":\"queued\"");
        let rounds = table::wait_rounds_to_view(world, mark);
        assert_eq!(rounds, [sit + 5], "travel ran twice");
        assert_eq!(model.calls().len(), 1, "{}", model.calls().len());
        assert_eq!(
            state_of(pack)
                .actors
                .get("pc_rowan")
                .map(|actor| actor.place.clone()),
            Some("docks_street".to_string())
        );
    });
}

fn epilogue_does_not_play_another_round(world: &mut World) {
    run_ai_gm(world, "host-epilogue", |world, pack, model| {
        model.push(Ok(EMPTY_ACT));
        model.push(Ok(
            r#"{"says":[{"address":"table","text":"the roll lands"}]}"#,
        ));
        table::seat_rowan(world, pack);
        let sit = state_of(pack).world_time;
        let (mark, _) = table::player_send(world, "grit", "\"kind\":\"queued\"");
        assert_eq!(table::wait_rounds_to_view(world, mark), [sit + 5]);
        table::wait_player_from(world, mark, "the roll lands");
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
        assert_eq!(
            state_of(pack).world_time,
            sit + 5,
            "epilogue played a round"
        );
        assert_eq!(model.calls().len(), 2);
    });
}

fn sit_commits_the_waiting_queue(world: &mut World) {
    let pack = copy_docks("host-sit");
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_rowan(world, &pack);
    table::player_send(world, "say good evening", "\"kind\":\"say\"");
    table::player_send(world, "travel docks_street", "\"kind\":\"queued\"");
    assert_eq!(state_of(&pack).world_time, 1080);
    table::release_gm(world);
    let model = ScriptedModel::default();
    model.push(Ok(EMPTY_ACT));
    let mark = table::player_mark(world);
    let gm = start_ai_gm(world, model.clone());
    assert_eq!(table::wait_rounds_to_view(world, mark), [1085]);
    assert_eq!(state_of(&pack).world_time, 1085);
    assert_eq!(model.calls().len(), 1);
    stop_ai_gm(world, gm);
    let _ = fs::remove_dir_all(&pack);
}

#[when(expr = "a player queues travel while the model answers their say")]
fn a_player_queues_travel_while_the_model_answers_their_say(world: &mut World) {
    let pack = copy_docks("host-close-turns");
    table::listen(world, &pack);
    let model = ScriptedModel::default();
    model.push(Ok(EMPTY_ACT));
    model.push(Ok(EMPTY_ACT));
    model.hold_next();
    let gm = start_ai_gm(world, model.clone());
    table::seat_rowan(world, &pack);
    let sit = state_of(&pack).world_time;
    table::player_send(world, "say hello", "\"kind\":\"say\"");
    table::poll_until("the say's model call", || match model.calls().len() {
        1 => Ok(()),
        calls => Err(calls),
    });
    let (mark, _) = table::player_send(world, "travel docks_street", "\"kind\":\"queued\"");
    gm.advance(HOLD);
    let rounds = table::wait_rounds_to_view(world, mark);
    assert_eq!(rounds.len(), 1, "{rounds:?}");
    let world_time = state_of(&pack).world_time;
    stop_ai_gm(world, gm);
    let _ = fs::remove_dir_all(&pack);
    world.mark = model.calls().len();
    world.stdout = if world_time == sit + 5 {
        "advanced".to_string()
    } else {
        world_time.to_string()
    };
}

/// How long a held model call waits on the AI GM's paused clock.
const HOLD: Duration = Duration::from_secs(1);

/// A model that answers each call with the next scripted `act` arguments, read through
/// `parse_completion` the way `HttpModel` reads a reply. With no script left it answers
/// without an `act` call.
#[derive(Clone, Default)]
struct ScriptedModel {
    script: Arc<Mutex<Script>>,
}

#[derive(Default)]
struct Script {
    replies: VecDeque<Result<&'static str, CompletionError>>,
    /// How many of the next calls wait `HOLD` before they answer.
    held: usize,
    calls: Vec<CompletionRequest>,
}

impl ScriptedModel {
    fn push(&self, reply: Result<&'static str, CompletionError>) {
        self.script.lock().unwrap().replies.push_back(reply);
    }

    fn hold_next(&self) {
        self.script.lock().unwrap().held += 1;
    }

    fn calls(&self) -> Vec<CompletionRequest> {
        self.script.lock().unwrap().calls.clone()
    }
}

impl Model for ScriptedModel {
    async fn complete(
        &mut self,
        request: &CompletionRequest,
    ) -> Result<Option<ModelReply>, CompletionError> {
        let (reply, held) = {
            let mut script = self.script.lock().unwrap();
            script.calls.push(request.clone());
            let held = script.held > 0;
            script.held = script.held.saturating_sub(1);
            (script.replies.pop_front(), held)
        };
        if held {
            tokio::time::sleep(HOLD).await;
        }
        match reply {
            Some(Ok(arguments)) => parse_completion(&act_reply(arguments), request.says_only),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }
}

fn run_ai_gm(world: &mut World, label: &str, body: impl FnOnce(&mut World, &Path, &ScriptedModel)) {
    let pack = copy_docks(label);
    table::listen(world, &pack);
    let model = ScriptedModel::default();
    let gm = start_ai_gm(world, model.clone());
    body(world, &pack, &model);
    stop_ai_gm(world, gm);
    let _ = fs::remove_dir_all(&pack);
}

/// `ai_gm::run` on its own thread, on a current-thread runtime whose clock starts paused.
/// A blocking task held for the run keeps tokio from auto-advancing, so paused time moves
/// only on `advance`.
struct AiGm {
    /// What the run wrote to `err`, and then its error if it returned one, as `spook ai-gm`
    /// prints it.
    err: Arc<Mutex<String>>,
    advance: tokio::sync::mpsc::UnboundedSender<table::Advance>,
    thread: thread::JoinHandle<Result<(), spookery_client::Error>>,
}

impl AiGm {
    /// Moves the AI GM's paused clock forward, after the AI GM has read what was ready for
    /// it. Timers due by then have fired when this returns.
    fn advance(&self, by: Duration) {
        let ended = || panic!("the AI GM ended\n{}", support::text_of(&self.err));
        let (ack, acked) = tokio::sync::oneshot::channel();
        self.advance.send((by, ack)).unwrap_or_else(|_| ended());
        support::block_on(acked).unwrap_or_else(|_| ended());
    }
}

/// Sits `model` as the AI GM and returns once it has written `listening`.
fn start_ai_gm(world: &World, mut model: impl Model + Send + 'static) -> AiGm {
    let addr: SocketAddr = table::game_addr(world).parse().unwrap();
    let (read, mut write) = std::io::pipe().unwrap_or_else(|err| panic!("pipe: {err}"));
    let err = Arc::new(Mutex::new(String::new()));
    support::drain(read, Arc::clone(&err));
    let (advance, mut advances) = tokio::sync::mpsc::unbounded_channel::<table::Advance>();
    let thread = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap_or_else(|err| panic!("runtime: {err}"));
        let result = runtime.block_on(async {
            let (_hold, held) = std::sync::mpsc::channel::<()>();
            tokio::task::spawn_blocking(move || held.recv());
            let run = ai_gm::run(addr, &mut model, &mut write);
            tokio::pin!(run);
            loop {
                tokio::select! {
                    // `run` first, so an advance lands after it has read what is ready.
                    biased;
                    result = &mut run => break result,
                    Some((by, ack)) = advances.recv() => {
                        tokio::time::advance(by).await;
                        let _ = ack.send(());
                    }
                }
            }
        });
        if let Err(err) = &result {
            let _ = writeln!(write, "{err}");
        }
        result
    });
    let gm = AiGm {
        err,
        advance,
        thread,
    };
    table::wait_suffix(&gm.err, 0, "listening");
    gm
}

fn stop_ai_gm(world: &mut World, gm: AiGm) {
    table::live_mut(world).host.take();
    let _ = gm.thread.join();
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

const SAY_ARGUMENTS: &str =
    r#"{"says":[{"address":"table","text":"STUB-SAY-FIXTURE"}],"intents":[]}"#;

const EMPTY_ACT: &str = r#"{"says":[],"intents":[]}"#;

#[when(expr = "a hung model call meets a queued player line")]
fn a_hung_model_call_meets_a_queued_player_line(world: &mut World) {
    let (pack, sit) = queued_travel(world, "host-hang");
    let (stub, seen) = start_model_stub([Reply::Hang, Reply::Hang]);
    let gm = start_ai_gm(world, http_model(stub));
    // The paused clock moves only here: past each call's limit, and past the pause between.
    wait_stub(&seen, 1, 0);
    gm.advance(MODEL_CALL_TIMEOUT);
    wait_stub(&seen, 1, 1);
    assert_eq!(
        seen.lock().unwrap().posts.len(),
        1,
        "retried before the delay"
    );
    gm.advance(RETRY_BACKOFF);
    wait_stub(&seen, 2, 1);
    gm.advance(MODEL_CALL_TIMEOUT);
    wait_stub(&seen, 2, 2);
    table::wait_round(world, sit + 5);
    let up = !gm.thread.is_finished();
    stop_ai_gm(world, gm);
    let _ = fs::remove_dir_all(&pack);
    let seen = seen.lock().unwrap();
    world.stdout = "advanced".to_string();
    world.stderr = if up { "up" } else { "down" }.to_string();
    world.mark = seen.posts.len();
    world.scratch = seen.closes.to_string();
}

#[when(expr = "a model answers 500 once and then a say")]
fn a_model_answers_500_once_and_then_a_say(world: &mut World) {
    let (pack, sit) = queued_travel(world, "host-retry");
    let replies = [
        Reply::Status(500, "{\"error\":\"busy\"}"),
        Reply::Act(SAY_ARGUMENTS),
    ];
    let (stub, seen) = start_model_stub(replies);
    let gm = start_ai_gm(world, http_model(stub));
    // Its socket closes once the 500 is read, so this lands in the pause before the retry.
    wait_stub(&seen, 1, 1);
    gm.advance(RETRY_BACKOFF);
    finish_http_turn(world, gm, &pack, sit, &seen);
}

#[when(expr = "a model answers 401")]
fn a_model_answers_401(world: &mut World) {
    let (pack, sit) = queued_travel(world, "host-unauthorized");
    let (stub, seen) = start_model_stub([Reply::Status(401, UNAUTHORIZED_BODY)]);
    let gm = start_ai_gm(world, http_model(stub));
    table::wait_suffix(&gm.err, 0, "model call returned 401");
    finish_http_turn(world, gm, &pack, sit, &seen);
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

#[then(expr = "the queued line committed one round")]
fn the_queued_line_committed_one_round(world: &mut World) {
    assert_eq!(world.stdout, "advanced", "{}", world.scratch);
}

#[then(expr = "the hung call retried once after the delay")]
fn the_hung_call_retried_once_after_the_delay(world: &mut World) {
    assert_eq!(world.mark, 2, "posts");
}

#[then(expr = "both hung calls closed")]
fn both_hung_calls_closed(world: &mut World) {
    assert_eq!(world.scratch, "2", "closes");
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

/// Waits for the queued travel's round, stops the AI GM, and keeps what the turn showed.
fn finish_http_turn(world: &mut World, gm: AiGm, pack: &Path, sit: u64, seen: &Mutex<Seen>) {
    table::wait_round(world, sit + 5);
    let tape = tape_text(pack);
    let err = support::text_of(&gm.err);
    stop_ai_gm(world, gm);
    let _ = fs::remove_dir_all(pack);
    let posts = seen.lock().unwrap().posts.clone();
    for post in &posts {
        assert_chat_post(post);
    }
    world.stdout = "advanced".to_string();
    world.stderr = err;
    world.mark = posts.len();
    world.scratch = if tape.contains("STUB-SAY-FIXTURE") {
        "said".to_string()
    } else {
        "silent".to_string()
    };
}

fn queued_travel(world: &mut World, label: &str) -> (PathBuf, u64) {
    let pack = copy_docks(label);
    table::listen(world, &pack);
    table::seat_gm(world);
    table::seat_rowan(world, &pack);
    let sit = state_of(&pack).world_time;
    table::player_send(world, "travel docks_street", "\"kind\":\"queued\"");
    assert_eq!(
        state_of(&pack).world_time,
        sit,
        "travel committed before the host"
    );
    table::release_gm(world);
    (pack, sit)
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
    /// Answer 200 with one `act` call that carries these arguments.
    Act(&'static str),
}

/// What the model stub has seen: each request in order, and how many of their connections
/// the client has closed.
#[derive(Default)]
struct Seen {
    posts: Vec<String>,
    closes: usize,
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
                Reply::Act(arguments) => Some((200, act_reply(arguments))),
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
fn wait_stub(seen: &Mutex<Seen>, posts: usize, closes: usize) {
    table::poll_until(&format!("{posts} posts and {closes} closes"), || {
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
    });
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
