//! The Spookery AI GM. `run` sits the GM seat and plays each turn through a graph that
//! awaits two adapters the client implements: a `Model` and a `Seat`. It reads the seat's
//! one ordered stream and decides from everything up to the last `seq` it read.

mod model;
mod prompt;
mod turn;

use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use graph_flow::GraphError;
use serde_json::{Map, Value};
use spookery_protocol as protocol;

pub use crate::model::{
    chat_body, parse_completion, provider_detail, ChatMessage, CompletionError, FailureKind, Model,
    ToolCall, MAX_DETAIL_CHARS,
};
use crate::prompt::{keep_recent, render_prompt, render_view, Opening, TapeLine, Wake};
use crate::protocol::{
    ClientBody, ClientMessage, PackSummary, QueuedMove, Refusal, ServerBody, ServerMessage,
    Speaker, TableStatus,
};

/// The most tool calls one turn makes, valid or not, `end_turn` included.
pub const MAX_TOOL_CALLS: usize = 20;

/// The GM seat's one ordered stream, as the client carries it.
pub trait Seat: Send + Sync + 'static {
    fn send(&self, message: ClientMessage) -> impl Future<Output = Result<(), SeatError>> + Send;
    /// The seat's next message. `None` when the host closed it.
    fn recv(&self) -> impl Future<Output = Option<Result<ServerMessage, SeatError>>> + Send;
}

/// Returned from a client's `Seat` to the agent: why the seat could not send or read.
/// Display is the client's line for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatError(pub String);

impl fmt::Display for SeatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why `run` stopped before the host closed the seat. Display is the line the client prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The `status` reply has no pack summary: a server built before the summary moved
    /// onto `status`.
    OlderServer,
    /// The table refused the `status` the agent sits with. Display is the refusal's body.
    Refused(Refusal),
    /// The `status` reply was some other body.
    Unexpected,
    /// The seat failed between turns.
    Seat(SeatError),
    /// `word` was sent and its reply never read, so the table may have applied it.
    Unanswered { word: String },
    /// The turn graph stopped with nothing in flight on the seat.
    Turn(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OlderServer => {
                f.write_str("this spook run is older than spook ai-gm, rebuild both")
            }
            Error::Refused(refusal) => {
                f.write_str(&serde_json::to_string(refusal).map_err(|_| fmt::Error)?)
            }
            Error::Unexpected => f.write_str("unexpected status reply"),
            Error::Seat(err) => err.fmt(f),
            Error::Unanswered { word } => write!(
                f,
                "no reply was read to {word}, so the table may have applied it; restart spook ai-gm to read the table again"
            ),
            Error::Turn(detail) => write!(f, "the GM turn stopped: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

/// Sits the GM seat and plays turns until the host closes it. `report` gets "listening"
/// and each warning, one line each.
pub async fn run<M: Model, S: Seat>(
    model: M,
    seat: S,
    report: impl Fn(&str) + Send + Sync + 'static,
) -> Result<(), Error> {
    let shared = Arc::new(Shared {
        model,
        seat,
        report: Box::new(report),
        table: Mutex::default(),
        turn: Mutex::default(),
        next_id: AtomicU64::new(0),
        in_flight: Mutex::new(None),
        closed: AtomicBool::new(false),
    });
    let graph = turn::graph(&shared);
    let Some((_, body)) = shared.send_word("status", Map::new()).await else {
        return shared.end_run("no reply to status");
    };
    match body {
        ServerBody::Status(status) => shared.table().sit(*status)?,
        ServerBody::Refused(refusal) => return Err(Error::Refused(refusal)),
        _ => return Err(Error::Unexpected),
    }
    (shared.report)("listening");
    loop {
        let turn = shared.table().start_turn();
        match turn {
            Some(turn) => {
                if let Err(err) = turn::play(&graph, &shared, turn).await {
                    return shared.end_run(err);
                }
            }
            None => match shared.seat.recv().await {
                Some(Ok(message)) => shared.table().record(&message),
                Some(Err(err)) => return Err(Error::Seat(err)),
                None => return Ok(()),
            },
        }
    }
}

/// What `run` and the turn graph's tasks share: the adapters, the table as read so far,
/// the turn being played, and what the seat is waiting on.
struct Shared<M, S> {
    model: M,
    seat: S,
    report: Box<dyn Fn(&str) + Send + Sync>,
    table: Mutex<Table>,
    turn: Mutex<Turn>,
    /// The number in the last message id.
    next_id: AtomicU64,
    /// The word sent whose reply has not been read.
    in_flight: Mutex<Option<String>>,
    /// Set once the host closed the seat.
    closed: AtomicBool,
}

impl<M: Model, S: Seat> Shared<M, S> {
    fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().expect("table")
    }

    fn turn(&self) -> MutexGuard<'_, Turn> {
        self.turn.lock().expect("turn")
    }

    /// Sends `word` and reads until its reply, recording every message on the way. `None`
    /// when the reply can't be read: the host closed the seat, or the word is now in flight.
    async fn send_word(&self, word: &str, args: Map<String, Value>) -> Option<(u64, ServerBody)> {
        let id = format!("a{}", self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let message = ClientMessage {
            id: Some(id.clone()),
            seq: None,
            reply_to: None,
            body: ClientBody {
                word: word.to_string(),
                args,
            },
        };
        *self.in_flight.lock().expect("in flight") = Some(word.to_string());
        self.seat.send(message).await.ok()?;
        loop {
            let message = match self.seat.recv().await {
                Some(Ok(message)) => message,
                Some(Err(_)) => return None,
                None => {
                    self.closed.store(true, Ordering::Relaxed);
                    return None;
                }
            };
            self.table().record(&message);
            if message.reply_to.as_deref() == Some(id.as_str()) {
                self.in_flight.lock().expect("in flight").take();
                return Some((message.seq.unwrap_or_default(), message.body));
            }
        }
    }

    /// Sends a tool call's `word` and returns the reply as the model reads it. A reply that
    /// can't be read stops the turn graph, and `end_run` says why.
    async fn relay(&self, word: &str, args: Map<String, Value>) -> graph_flow::Result<String> {
        let lost = || GraphError::TaskExecutionFailed(format!("no reply to {word}"));
        let (seq, body) = self.send_word(word, args).await.ok_or_else(lost)?;
        let table = self.table();
        Ok(self.turn().read_reply(&table.pack, seq, body))
    }

    /// Plays the round: `commit` with its rounds, for `end_turn` and for `close`.
    async fn commit(&self) -> graph_flow::Result<String> {
        let args = Map::from_iter([("include_round".to_string(), Value::Bool(true))]);
        self.relay("commit", args).await
    }

    /// How `run` ends once the seat or the graph stopped with `detail`: quietly when the host
    /// closed the seat, and otherwise naming the word in flight.
    fn end_run(&self, detail: impl fmt::Display) -> Result<(), Error> {
        if self.closed.load(Ordering::Relaxed) {
            return Ok(());
        }
        Err(match self.in_flight.lock().expect("in flight").take() {
            Some(word) => Error::Unanswered { word },
            None => Error::Turn(detail.to_string()),
        })
    }
}

/// The table as the seat's stream has shown it so far, and the turns it wakes.
#[derive(Default)]
struct Table {
    pack: PackSummary,
    /// Who is seated, as of the last message read.
    present: Vec<String>,
    /// The queue, as of the last message read.
    queue: Vec<QueuedMove>,
    /// Player says read since a turn last took them.
    heard: Vec<TapeLine>,
    wakes: VecDeque<Wake>,
    /// The recent tape every opening prompt shows.
    buffer: Vec<TapeLine>,
}

/// One turn: the conversation with the model and what the commit rule reads.
#[derive(Default)]
struct Turn {
    messages: Vec<ChatMessage>,
    /// A move waits for a commit: the turn saw a queue when it started, or queued a move
    /// itself since its last commit.
    move_waiting: bool,
    /// The player says this turn shows the model.
    lines: Vec<TapeLine>,
    /// The GM's says this turn.
    said: Vec<TapeLine>,
}

impl Turn {
    /// One reply as the model reads it, once the turn has noted what it changed.
    fn read_reply(&mut self, pack: &PackSummary, seq: u64, body: ServerBody) -> String {
        match body {
            ServerBody::View(view) => return render_view(pack, *view),
            ServerBody::Queued { .. } => self.move_waiting = true,
            ServerBody::Committed(_) => self.move_waiting = false,
            ServerBody::Say(ref said) => self.said.push(TapeLine::new(seq, said.clone())),
            _ => {}
        }
        serde_json::to_string(&body).expect("server body")
    }
}

impl Table {
    fn sit(&mut self, mut status: TableStatus) -> Result<(), Error> {
        self.pack = status.pack.take().ok_or(Error::OlderServer)?;
        self.present = status.presence.actors_present;
        self.queue = status.queued;
        if !self.queue.is_empty() && !self.wakes.contains(&Wake::Queued) {
            self.wakes.push_front(Wake::Queued);
        }
        Ok(())
    }

    /// What one message says about the table: a player's say wakes a turn, a queued move
    /// joins the queue and wakes one, a round empties the queue, and presence is who sits.
    fn record(&mut self, message: &ServerMessage) {
        match &message.body {
            ServerBody::Say(say) if matches!(say.from, Speaker::Player(_)) => {
                let line = TapeLine::new(message.seq.unwrap_or_default(), say.clone());
                self.heard.push(line.clone());
                self.wakes.push_back(Wake::Say(line));
            }
            ServerBody::Queued {
                intent,
                actor,
                move_name,
                ..
            } => {
                self.queue.push(QueuedMove {
                    id: intent.clone(),
                    actor_id: actor.clone(),
                    move_name: move_name.clone(),
                });
                if !self.wakes.contains(&Wake::Queued) {
                    self.wakes.push_back(Wake::Queued);
                }
            }
            ServerBody::Round { .. } => {
                self.queue.clear();
                self.wakes.retain(|wake| *wake != Wake::Queued);
            }
            ServerBody::Presence { actors_present } => self.present = actors_present.clone(),
            _ => {}
        }
    }

    /// The next turn, when a wake waits: its opening prompt and the new player lines.
    fn start_turn(&mut self) -> Option<Turn> {
        let wake = self.wakes.pop_front()?;
        let lines = std::mem::take(&mut self.heard);
        let prompt = render_prompt(&Opening {
            menu: &self.pack.menu,
            buffer: &self.buffer,
            lines: &lines,
            queue: &self.queue,
            wake: &wake,
            seated: &self.present,
        });
        Some(Turn {
            messages: vec![ChatMessage::User { content: prompt }],
            move_waiting: !self.queue.is_empty(),
            lines,
            said: Vec::new(),
        })
    }

    /// The turn's lines join the buffer, and a say the model has now read wakes no turn.
    fn finish_turn(&mut self, turn: Turn) {
        let seen = turn.lines.iter().map(|line| line.seq).max();
        for line in turn.lines.into_iter().chain(turn.said) {
            keep_recent(&mut self.buffer, line);
        }
        if let Some(seen) = seen {
            self.wakes.retain(|wake| match wake {
                Wake::Say(line) => line.seq > seen,
                Wake::Queued => true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const QUEUED_STATUS: &str = r#"{"kind":"status","presence":{"players":1,"actors_present":["pc"]},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[{"id":"q1","actor_id":"pc","move":"wait"}],"pack":{"menu":[],"kinds":{"pc":"pc_template"},"places":{},"cast":{}}}"#;
    const EMPTY_STATUS: &str = r#"{"kind":"status","presence":{"players":1,"actors_present":["pc"]},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[],"pack":{"menu":[],"kinds":{"pc":"pc_template"},"places":{},"cast":{}}}"#;
    const OLD_STATUS: &str = r#"{"kind":"status","presence":{"players":1,"actors_present":["pc"]},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[]}"#;
    const VIEW: &str = r#"{"kind":"view","viewer":"pc","place":"docks","visible_actors":["pc"],"known_facts":[],"notes":[],"unknown_mentions":[],"items":[],"habits":[]}"#;
    const COMMITTED: &str = r#"{"kind":"committed","state_rev":2,"changed_keys":[],"journal_path":null,"rolls":[{"faces":[6,6],"stat":1,"total":13,"target":10,"outcome":"success"}]}"#;
    const ROUND: &str = r#"{"kind":"round","round_index":1,"world_time":5}"#;
    const QUEUED: &str = r#"{"kind":"queued","intent":"q2","actor":"pc","move":"wait"}"#;
    const REFUSED: &str = r#"{"kind":"refused","code":"UNKNOWN_ID"}"#;
    const PLAYER_SAY: &str = r#"{"kind":"say","from":"player:pc","address":"table","text":"ok"}"#;
    const GM_SAY: &str = r#"{"kind":"say","from":"gm","address":"table","text":"ok"}"#;
    /// Read as a failed read rather than as a body.
    const BROKEN: &str = "broken";
    /// Read as a read that never ends.
    const SILENT: &str = "silent";

    const PERCEIVE: (&str, &str) = ("perceive", r#"{"as":"pc"}"#);
    const SAY: (&str, &str) = ("say", r#"{"address":"table","text":"ok"}"#);
    const QUEUE: (&str, &str) = ("queue", r#"{"actor_id":"pc","move":"wait"}"#);
    const END_TURN: (&str, &str) = ("end_turn", "{}");

    /// What the scripted table sends for one word: `bodies`, the last of them the reply, and
    /// then `then`, which arrive while the agent waits.
    struct Answer {
        word: &'static str,
        bodies: Vec<&'static str>,
        then: Vec<&'static str>,
    }

    fn answer(word: &'static str, bodies: &[&'static str]) -> Answer {
        Answer {
            word,
            bodies: bodies.to_vec(),
            then: Vec::new(),
        }
    }

    impl Answer {
        fn then(mut self, bodies: &[&'static str]) -> Self {
            self.then = bodies.to_vec();
            self
        }
    }

    /// A table that answers each word in the order scripted, and closes once nothing is
    /// left to read.
    #[derive(Default)]
    struct ScriptSeat {
        answers: Mutex<VecDeque<Answer>>,
        inbox: Mutex<VecDeque<ServerMessage>>,
        words: Mutex<Vec<String>>,
        seq: AtomicU64,
    }

    impl ScriptSeat {
        fn new(answers: Vec<Answer>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers.into()),
                ..Self::default()
            })
        }

        fn push(&self, body: &str, reply_to: Option<&str>) {
            let body = match body {
                BROKEN | SILENT => json!({"kind": "lines", "text": body}),
                body => serde_json::from_str(body).unwrap(),
            };
            self.inbox.lock().unwrap().push_back(ServerMessage {
                id: None,
                seq: Some(self.seq.fetch_add(1, Ordering::Relaxed) + 1),
                reply_to: reply_to.map(str::to_string),
                body: serde_json::from_value(body).unwrap(),
            });
        }

        fn words(&self) -> Vec<String> {
            self.words.lock().unwrap().clone()
        }
    }

    impl Seat for Arc<ScriptSeat> {
        async fn send(&self, message: ClientMessage) -> Result<(), SeatError> {
            let answer = self.answers.lock().unwrap().pop_front();
            let answer = answer.unwrap_or_else(|| panic!("unscripted {:?}", message.body));
            assert_eq!(message.body.word, answer.word, "{:?}", message.body);
            let (reply, before) = answer.bodies.split_last().unwrap();
            for body in before {
                self.push(body, None);
            }
            self.push(reply, message.id.as_deref());
            for body in &answer.then {
                self.push(body, None);
            }
            self.words.lock().unwrap().push(message.body.word);
            Ok(())
        }

        async fn recv(&self) -> Option<Result<ServerMessage, SeatError>> {
            let message = self.inbox.lock().unwrap().pop_front()?;
            match &message.body {
                ServerBody::Lines { text } if text == BROKEN => {
                    Some(Err(SeatError("reset".to_string())))
                }
                ServerBody::Lines { text } if text == SILENT => std::future::pending().await,
                _ => Some(Ok(message)),
            }
        }
    }

    type Reply = Result<Vec<(&'static str, &'static str)>, CompletionError>;

    /// A model that answers each call with the next scripted tool calls, read through
    /// `parse_completion`, and with `repeat` once they run out.
    #[derive(Default)]
    struct ScriptModel {
        replies: Mutex<VecDeque<Reply>>,
        repeat: Vec<(&'static str, &'static str)>,
        calls: Mutex<Vec<Vec<ChatMessage>>>,
    }

    impl ScriptModel {
        fn new(replies: Vec<Reply>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.into()),
                ..Self::default()
            })
        }

        fn calls(&self) -> Vec<Vec<ChatMessage>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Model for Arc<ScriptModel> {
        async fn complete(
            &self,
            messages: &[ChatMessage],
        ) -> Result<Vec<ToolCall>, CompletionError> {
            self.calls.lock().unwrap().push(messages.to_vec());
            let reply = self.replies.lock().unwrap().pop_front();
            let tools = reply.unwrap_or_else(|| Ok(self.repeat.clone()))?;
            let calls: Vec<Value> = tools
                .iter()
                .enumerate()
                .map(|(index, (name, arguments))| {
                    json!({"id": format!("call_{index}"), "type": "function",
                        "function": {"name": name, "arguments": arguments}})
                })
                .collect();
            let reply =
                json!({"choices": [{"message": {"role": "assistant", "tool_calls": calls}}]});
            parse_completion(&reply.to_string())
        }
    }

    /// Runs the agent until the scripted table closes, and returns its result and reports.
    async fn play(
        model: &Arc<ScriptModel>,
        seat: &Arc<ScriptSeat>,
    ) -> (Result<(), Error>, Vec<String>) {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let lines = Arc::clone(&reports);
        let report = move |line: &str| lines.lock().unwrap().push(line.to_string());
        let result = run(Arc::clone(model), Arc::clone(seat), report).await;
        let reports = reports.lock().unwrap().clone();
        (result, reports)
    }

    /// The tool messages of one model call, in order, as (call id, content).
    fn read_tool_results(messages: &[ChatMessage]) -> Vec<(String, String)> {
        messages
            .iter()
            .filter_map(|message| match message {
                ChatMessage::Tool {
                    tool_call_id,
                    content,
                } => Some((tool_call_id.clone(), content.clone())),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_is_the_tool_result_and_the_turn_recovers() {
        let model = ScriptModel::new(vec![
            Ok(vec![("queue", r#"{"actor_id":"pc","move":"fly"}"#)]),
            Ok(vec![QUEUE]),
            Ok(vec![END_TURN]),
            Ok(vec![SAY]),
        ]);
        let seat = ScriptSeat::new(vec![
            answer("status", &[EMPTY_STATUS]).then(&[PLAYER_SAY]),
            answer("queue", &[REFUSED]),
            answer("queue", &[QUEUED]),
            answer("commit", &[ROUND, COMMITTED]),
            answer("say", &[GM_SAY]),
        ]);
        let (result, reports) = play(&model, &seat).await;
        assert_eq!(result, Ok(()));
        assert_eq!(reports, ["listening"]);
        let calls = model.calls();
        assert_eq!(calls.len(), 5);
        let results: Vec<String> = read_tool_results(&calls[4])
            .into_iter()
            .map(|r| r.1)
            .collect();
        assert!(results[0].contains("\"refused\""), "{results:?}");
        assert!(results[1].contains("\"queued\""), "{results:?}");
        assert!(results[2].contains("\"rolls\""), "{results:?}");
        assert!(results[3].contains("\"say\""), "{results:?}");
        assert_eq!(seat.words(), ["status", "queue", "queue", "commit", "say"]);
    }

    #[tokio::test(start_paused = true)]
    async fn every_call_in_a_reply_is_answered_before_the_next_ask() {
        let model = ScriptModel::new(vec![Ok(vec![
            PERCEIVE,
            ("dance", "{}"),
            ("say", "[1]"),
            END_TURN,
        ])]);
        let seat = ScriptSeat::new(vec![
            answer("status", &[EMPTY_STATUS]).then(&[PLAYER_SAY]),
            answer("perceive", &[VIEW]),
        ]);
        let (result, _) = play(&model, &seat).await;
        assert_eq!(result, Ok(()));
        let calls = model.calls();
        assert_eq!(calls.len(), 2);
        let results = read_tool_results(&calls[1]);
        let ids: Vec<&str> = results.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["call_0", "call_1", "call_2", "call_3"]);
        assert!(results[0].1.contains("\"viewer\":\"pc\""), "{results:?}");
        assert!(results[1].1.contains("no tool named dance"), "{results:?}");
        assert!(results[2].1.contains("not a JSON object"), "{results:?}");
        // Nothing was queued, so end_turn sends no word.
        assert!(results[3].1.contains("nothing is queued"), "{results:?}");
        assert_eq!(seat.words(), ["status", "perceive"]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_step_limit_commits_and_warns() {
        let model = Arc::new(ScriptModel {
            repeat: vec![PERCEIVE],
            ..ScriptModel::default()
        });
        let mut answers = vec![answer("status", &[QUEUED_STATUS])];
        answers.extend((0..MAX_TOOL_CALLS).map(|_| answer("perceive", &[VIEW])));
        answers.push(answer("commit", &[ROUND, COMMITTED]));
        let seat = ScriptSeat::new(answers);
        let (result, reports) = play(&model, &seat).await;
        assert_eq!(result, Ok(()));
        assert_eq!(model.calls().len(), MAX_TOOL_CALLS);
        assert_eq!(seat.words().last().map(String::as_str), Some("commit"));
        assert!(reports[1].starts_with("step limit"), "{reports:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_move_queued_during_a_say_turn_waits_for_its_own_turn() {
        let model = ScriptModel::new(vec![Ok(vec![SAY]), Ok(Vec::new()), Ok(Vec::new())]);
        let seat = ScriptSeat::new(vec![
            answer("status", &[EMPTY_STATUS]).then(&[PLAYER_SAY]),
            // Read mid-turn, so this turn's opening never showed it.
            answer("say", &[QUEUED, GM_SAY]),
            answer("commit", &[ROUND, COMMITTED]),
        ]);
        let (result, _) = play(&model, &seat).await;
        assert_eq!(result, Ok(()));
        // The say turn does not commit. The queued move starts the next turn, which does.
        assert_eq!(model.calls().len(), 3);
        assert_eq!(seat.words(), ["status", "say", "commit"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_round_the_agent_did_not_play_drops_the_queued_wake() {
        let model = ScriptModel::new(vec![Ok(vec![SAY])]);
        let seat = ScriptSeat::new(vec![
            answer("status", &[EMPTY_STATUS]).then(&[PLAYER_SAY]),
            answer("say", &[QUEUED, ROUND, GM_SAY]),
        ]);
        let (result, _) = play(&model, &seat).await;
        assert_eq!(result, Ok(()));
        // Someone else's round played that move, so no turn waits for it.
        assert_eq!(model.calls().len(), 2);
        assert_eq!(seat.words(), ["status", "say"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_model_error_is_a_warning_and_the_waiting_move_commits() {
        let model = ScriptModel::new(vec![Err(CompletionError::Transport("boom".into()))]);
        let seat = ScriptSeat::new(vec![
            answer("status", &[QUEUED_STATUS]),
            answer("commit", &[ROUND, COMMITTED]),
        ]);
        let (result, reports) = play(&model, &seat).await;
        assert_eq!(result, Ok(()));
        assert_eq!(reports, ["listening", "boom"]);
        assert_eq!(seat.words(), ["status", "commit"]);
    }

    /// A read that fails, and a read the graph's task timeout cuts off.
    #[tokio::test(start_paused = true)]
    async fn a_lost_reply_stops_the_agent_naming_the_word() {
        for lost in [BROKEN, SILENT] {
            let model = ScriptModel::new(vec![Ok(vec![PERCEIVE, SAY])]);
            let seat = ScriptSeat::new(vec![
                answer("status", &[QUEUED_STATUS]),
                answer("perceive", &[lost, VIEW]),
            ]);
            let (result, _) = play(&model, &seat).await;
            let word = "perceive".to_string();
            assert_eq!(result, Err(Error::Unanswered { word }), "{lost}");
            // No retry, no commit and no further words.
            assert_eq!(seat.words(), ["status", "perceive"], "{lost}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_status_without_a_pack_summary_is_an_older_server() {
        let model = ScriptModel::new(Vec::new());
        let seat = ScriptSeat::new(vec![answer("status", &[OLD_STATUS])]);
        assert_eq!(play(&model, &seat).await.0, Err(Error::OlderServer));
    }
}
