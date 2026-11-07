//! The Spookery AI GM. `run` connects as the GM and plays each turn through a graph that
//! awaits two adapters the client implements: a `Model` and a `Connection`. It reads the
//! server's one ordered stream and decides from everything up to the last `seq` it read.

mod model;
mod prompt;
mod turn;

use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use graph_flow::GraphError;
use serde_json::{json, Value};
use spookery_protocol as protocol;

pub use crate::model::{
    parse_completion, provider_detail, render_chat_body, ChatMessage, CompletionError, FailureKind,
    Model, ToolCall, MAX_DETAIL_CHARS,
};
use crate::prompt::{keep_recent, render_prompt, Opening, TapeLine, Wake};
use crate::protocol::{
    ClientMessage, PackSummary, QueuedMove, Refusal, ServerBody, ServerMessage, Speaker,
    TableStatus,
};

/// The most tool calls one turn makes, valid or not, `end_turn` included.
pub const MAX_TOOL_CALLS: usize = 20;

/// The GM's connection to the server: the server's one ordered stream of messages, as the
/// client carries it.
pub trait Connection: Send + Sync + 'static {
    fn send(
        &self,
        message: ClientMessage,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send;
    /// The server's next message. `None` when the server closed the connection.
    fn recv(&self) -> impl Future<Output = Option<Result<ServerMessage, ConnectionError>>> + Send;
}

/// Returned from a client's `Connection` to the agent: why it could not send or read.
/// Display is the client's line for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionError(pub String);

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why `run` stopped before the server closed the connection. Display is the line the client prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The server refused the agent's first `status`. Display is the refusal's body.
    Refused(Refusal),
    /// The `status` reply was some other body.
    Unexpected,
    /// The connection failed between turns.
    Connection(ConnectionError),
    /// `command` was sent and its reply never read, so the server may have applied it.
    Unanswered { command: String },
    /// The turn graph stopped with no command in flight.
    Graph(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Refused(refusal) => {
                f.write_str(&serde_json::to_string(refusal).map_err(|_| fmt::Error)?)
            }
            Error::Unexpected => f.write_str("unexpected status reply"),
            Error::Connection(err) => err.fmt(f),
            Error::Unanswered { command } => write!(
                f,
                "no reply was read to {command}, so the server may have applied it; restart spook ai-gm to read the server's state again"
            ),
            Error::Graph(detail) => write!(f, "the GM turn stopped: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

/// Connects as the GM and plays turns until the server closes the connection. `report` gets "listening"
/// and each warning, one line each.
pub async fn run<M: Model, C: Connection>(
    model: M,
    connection: C,
    report: impl Fn(&str) + Send + Sync + 'static,
) -> Result<(), Error> {
    let agent = Arc::new(Agent {
        model,
        connection,
        report: Box::new(report),
        state: Mutex::default(),
        turn: Mutex::default(),
        last_id: AtomicU64::new(0),
        in_flight: Mutex::new(None),
        closed: AtomicBool::new(false),
    });
    let graph = turn::build_graph(&agent);
    match agent.send_command("status", json!({})).await {
        Ok(ServerBody::Status(status)) => agent.state().initialize(*status),
        Ok(ServerBody::Refused(refusal)) => return Err(Error::Refused(refusal)),
        Ok(_) => return Err(Error::Unexpected),
        Err(err) => return agent.end_run(err),
    }
    (agent.report)("listening");
    loop {
        let turn = agent.state().start_turn();
        match turn {
            Some(turn) => {
                if let Err(err) = turn::play(&graph, &agent, turn).await {
                    return agent.end_run(err);
                }
            }
            None => match agent.connection.recv().await {
                Some(Ok(message)) => agent.state().record(&message),
                Some(Err(err)) => return Err(Error::Connection(err)),
                None => return Ok(()),
            },
        }
    }
}

/// The AI GM on one connection: the adapters, the client state read so far, the turn being
/// played, and the command in flight. `run` and the turn graph's tasks hold it in an `Arc`.
struct Agent<M, C> {
    model: M,
    connection: C,
    report: Box<dyn Fn(&str) + Send + Sync>,
    state: Mutex<ClientState>,
    turn: Mutex<Turn>,
    /// The number in the last message id.
    last_id: AtomicU64,
    /// The command sent whose reply has not been read.
    in_flight: Mutex<Option<String>>,
    /// Set once the server closed the connection.
    closed: AtomicBool,
}

impl<M: Model, C: Connection> Agent<M, C> {
    fn state(&self) -> MutexGuard<'_, ClientState> {
        self.state.lock().expect("client state")
    }

    fn turn(&self) -> MutexGuard<'_, Turn> {
        self.turn.lock().expect("turn")
    }

    /// Sends `command` with `args`, a JSON object, and reads until its reply. Every message on
    /// the way goes to the client state, and the reply's bookkeeping to the turn. A reply
    /// that can't be read is an error, and `end_run` says why.
    async fn send_command(&self, command: &str, args: Value) -> graph_flow::Result<ServerBody> {
        let id = format!("a{}", self.last_id.fetch_add(1, Ordering::Relaxed) + 1);
        let lost = || GraphError::TaskExecutionFailed(format!("no reply to {command}"));
        *self.in_flight.lock().expect("in flight") = Some(command.to_string());
        let message = ClientMessage::new(&id, command, args);
        self.connection.send(message).await.map_err(|_| lost())?;
        loop {
            let message = match self.connection.recv().await {
                Some(Ok(message)) => message,
                Some(Err(_)) => return Err(lost()),
                None => {
                    self.closed.store(true, Ordering::Relaxed);
                    return Err(lost());
                }
            };
            self.state().record(&message);
            if message.reply_to.as_deref() == Some(id.as_str()) {
                self.in_flight.lock().expect("in flight").take();
                let seq = message.seq.unwrap_or_default();
                self.turn().record_reply(seq, &message.body);
                return Ok(message.body);
            }
        }
    }

    /// How `run` ends once the connection or the graph stopped with `detail`: quietly when
    /// the server closed the connection, and otherwise naming the command in flight.
    fn end_run(&self, detail: impl fmt::Display) -> Result<(), Error> {
        if self.closed.load(Ordering::Relaxed) {
            return Ok(());
        }
        Err(match self.in_flight.lock().expect("in flight").take() {
            Some(command) => Error::Unanswered { command },
            None => Error::Graph(detail.to_string()),
        })
    }
}

/// The server's state as its messages have shown this client so far, and the turns they
/// wake.
#[derive(Default)]
struct ClientState {
    pack: PackSummary,
    /// The actors present, as of the last message read.
    actors_present: Vec<String>,
    /// The queued moves, as of the last message read.
    queued_moves: Vec<QueuedMove>,
    /// Player says read since a turn last showed them to the model.
    new_player_lines: Vec<TapeLine>,
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
    player_lines: Vec<TapeLine>,
    /// The says the GM sent this turn, as the server confirmed them.
    gm_lines: Vec<TapeLine>,
}

impl Turn {
    /// Records the reply to one of the agent's commands: a `queued` reply means a move
    /// waits, a `committed` reply means none does, and the GM's confirmed say joins
    /// `gm_lines`. Every other reply leaves the turn as it is.
    fn record_reply(&mut self, seq: u64, body: &ServerBody) {
        match body {
            ServerBody::Queued { .. } => self.move_waiting = true,
            ServerBody::Committed(_) => self.move_waiting = false,
            ServerBody::Say(say) => self.gm_lines.push(TapeLine::new(seq, say.clone())),
            _ => {}
        }
    }
}

impl ClientState {
    /// Fills the state from the first `status`: the pack, the actors present and the queue,
    /// which wakes a turn when a move already waits.
    fn initialize(&mut self, status: TableStatus) {
        self.pack = status.pack;
        self.actors_present = status.presence.actors_present;
        self.queued_moves = status.queued;
        if !self.queued_moves.is_empty() && !self.wakes.contains(&Wake::QueuedMove) {
            self.wakes.push_front(Wake::QueuedMove);
        }
    }

    /// What one message says about the server's state: a player's say wakes a turn, a queued
    /// move joins the queued moves and wakes one, a round empties them, and presence names the
    /// actors present.
    fn record(&mut self, message: &ServerMessage) {
        match &message.body {
            ServerBody::Say(say) if matches!(say.from, Speaker::Player(_)) => {
                let line = TapeLine::new(message.seq.unwrap_or_default(), say.clone());
                self.new_player_lines.push(line.clone());
                self.wakes.push_back(Wake::Say(line));
            }
            ServerBody::Queued {
                intent,
                actor,
                move_name,
                ..
            } => {
                self.queued_moves.push(QueuedMove {
                    id: intent.clone(),
                    actor_id: actor.clone(),
                    move_name: move_name.clone(),
                });
                if !self.wakes.contains(&Wake::QueuedMove) {
                    self.wakes.push_back(Wake::QueuedMove);
                }
            }
            ServerBody::Round { .. } => {
                self.queued_moves.clear();
                self.wakes.retain(|wake| *wake != Wake::QueuedMove);
            }
            ServerBody::Presence { actors_present } => {
                self.actors_present = actors_present.clone();
            }
            _ => {}
        }
    }

    /// The next turn, when a wake waits: its opening prompt and the new player lines.
    fn start_turn(&mut self) -> Option<Turn> {
        let wake = self.wakes.pop_front()?;
        let player_lines = std::mem::take(&mut self.new_player_lines);
        let prompt = render_prompt(&Opening {
            menu: &self.pack.menu,
            buffer: &self.buffer,
            new_player_lines: &player_lines,
            queued_moves: &self.queued_moves,
            wake: &wake,
            actors_present: &self.actors_present,
        });
        Some(Turn {
            messages: vec![ChatMessage::Prompt { content: prompt }],
            move_waiting: !self.queued_moves.is_empty(),
            player_lines,
            gm_lines: Vec::new(),
        })
    }

    /// Records a played turn: its player and GM lines join the buffer, and a say the model
    /// has now read wakes no turn.
    fn record_turn(&mut self, turn: Turn) {
        let seen = turn.player_lines.iter().map(|line| line.seq).max();
        for line in turn.player_lines.into_iter().chain(turn.gm_lines) {
            keep_recent(&mut self.buffer, line);
        }
        if let Some(seen) = seen {
            self.wakes.retain(|wake| match wake {
                Wake::Say(line) => line.seq > seen,
                Wake::QueuedMove => true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = r#"{"kind":"status","presence":{"players":1,"actors_present":["pc"]},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[],"pack":{"menu":[],"kinds":{"pc":"pc_template"},"places":{},"cast":{}}}"#;
    const VIEW: &str = r#"{"kind":"view","viewer":"pc","place":"docks","visible_actors":["pc"],"known_facts":[],"notes":[],"unknown_mentions":[],"items":[],"habits":[]}"#;
    const ROUND: &str = r#"{"kind":"round","round_index":1,"world_time":5}"#;
    const QUEUED: &str = r#"{"kind":"queued","intent":"q2","actor":"pc","move":"wait"}"#;
    const PLAYER_SAY: &str = r#"{"kind":"say","from":"player:pc","address":"table","text":"ok"}"#;
    const GM_SAY: &str = r#"{"kind":"say","from":"gm","address":"table","text":"ok"}"#;
    /// Read as a failed read rather than as a body.
    const BROKEN: &str = "broken";
    /// Read as a read that never ends.
    const SILENT: &str = "silent";

    const PERCEIVE: (&str, &str) = ("perceive", r#"{"as":"pc"}"#);
    const SAY: (&str, &str) = ("say", r#"{"address":"table","text":"ok"}"#);

    /// What the scripted server sends for one command: `bodies`, the last of them the reply, and
    /// then `then`, which arrive while the agent waits.
    struct Answer {
        command: &'static str,
        bodies: Vec<&'static str>,
        then: Vec<&'static str>,
    }

    fn answer(command: &'static str, bodies: &[&'static str]) -> Answer {
        Answer {
            command,
            bodies: bodies.to_vec(),
            then: Vec::new(),
        }
    }

    /// The agent's first status, then a player's say that wakes a turn.
    fn status_then_wake() -> Answer {
        Answer {
            then: vec![PLAYER_SAY],
            ..answer("status", &[STATUS])
        }
    }

    /// A server that answers each command in the order scripted, and closes once nothing is
    /// left to read.
    #[derive(Default)]
    struct ScriptedConnection {
        answers: Mutex<VecDeque<Answer>>,
        inbox: Mutex<VecDeque<ServerMessage>>,
        commands: Mutex<Vec<String>>,
        seq: AtomicU64,
    }

    impl ScriptedConnection {
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

        fn commands(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }
    }

    impl Connection for Arc<ScriptedConnection> {
        async fn send(&self, message: ClientMessage) -> Result<(), ConnectionError> {
            let answer = self.answers.lock().unwrap().pop_front();
            let answer = answer.unwrap_or_else(|| panic!("unscripted {:?}", message.body));
            assert_eq!(message.body.kind, answer.command, "{:?}", message.body);
            let (reply, before) = answer.bodies.split_last().unwrap();
            for body in before {
                self.push(body, None);
            }
            self.push(reply, message.id.as_deref());
            for body in &answer.then {
                self.push(body, None);
            }
            self.commands.lock().unwrap().push(message.body.kind);
            Ok(())
        }

        async fn recv(&self) -> Option<Result<ServerMessage, ConnectionError>> {
            let message = self.inbox.lock().unwrap().pop_front()?;
            match &message.body {
                ServerBody::Lines { text } if text == BROKEN => {
                    Some(Err(ConnectionError("reset".to_string())))
                }
                ServerBody::Lines { text } if text == SILENT => std::future::pending().await,
                _ => Some(Ok(message)),
            }
        }
    }

    /// A model that answers each call with the next scripted tool calls, read through
    /// `parse_completion`, and with none once they run out.
    #[derive(Default)]
    struct ScriptedModel {
        replies: Mutex<VecDeque<Vec<(&'static str, &'static str)>>>,
        calls: Mutex<Vec<Vec<ChatMessage>>>,
    }

    impl ScriptedModel {
        fn new(replies: Vec<Vec<(&'static str, &'static str)>>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.into()),
                ..Self::default()
            })
        }

        fn calls(&self) -> Vec<Vec<ChatMessage>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Model for Arc<ScriptedModel> {
        async fn request_tool_calls(
            &self,
            messages: &[ChatMessage],
        ) -> Result<Vec<ToolCall>, CompletionError> {
            self.calls.lock().unwrap().push(messages.to_vec());
            let tools = self.replies.lock().unwrap().pop_front().unwrap_or_default();
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

    /// Runs the agent until the scripted server closes the connection.
    async fn play(
        model: &Arc<ScriptedModel>,
        connection: &Arc<ScriptedConnection>,
    ) -> Result<(), Error> {
        run(Arc::clone(model), Arc::clone(connection), |_| {}).await
    }

    #[tokio::test(start_paused = true)]
    async fn every_call_in_a_reply_is_answered_before_the_next_ask() {
        let model = ScriptedModel::new(vec![vec![
            PERCEIVE,
            ("dance", "{}"),
            ("say", "[1]"),
            ("end_turn", "{}"),
        ]]);
        let connection =
            ScriptedConnection::new(vec![status_then_wake(), answer("perceive", &[VIEW])]);
        assert_eq!(play(&model, &connection).await, Ok(()));
        let calls = model.calls();
        assert_eq!(calls.len(), 2);
        let results: Vec<(&str, &str)> = calls[1]
            .iter()
            .filter_map(|message| match message {
                ChatMessage::ToolResult {
                    tool_call_id,
                    content,
                } => Some((tool_call_id.as_str(), content.as_str())),
                _ => None,
            })
            .collect();
        let ids: Vec<&str> = results.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["call_0", "call_1", "call_2", "call_3"]);
        assert!(results[0].1.contains("\"viewer\":\"pc\""), "{results:?}");
        assert!(results[1].1.contains("no tool named dance"), "{results:?}");
        assert!(results[2].1.contains("not a JSON object"), "{results:?}");
        // Nothing was queued, so end_turn sends no command.
        assert!(results[3].1.contains("nothing is queued"), "{results:?}");
        assert_eq!(connection.commands(), ["status", "perceive"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_round_the_agent_did_not_play_drops_the_queued_wake() {
        let model = ScriptedModel::new(vec![vec![SAY]]);
        let connection = ScriptedConnection::new(vec![
            status_then_wake(),
            answer("say", &[QUEUED, ROUND, GM_SAY]),
        ]);
        assert_eq!(play(&model, &connection).await, Ok(()));
        // Someone else's round played that move, so no turn waits for it.
        assert_eq!(model.calls().len(), 2);
        assert_eq!(connection.commands(), ["status", "say"]);
    }

    /// A read that fails, and a read the graph's task timeout cuts off.
    #[tokio::test(start_paused = true)]
    async fn a_lost_reply_stops_the_agent_naming_the_command() {
        for lost in [BROKEN, SILENT] {
            let model = ScriptedModel::new(vec![vec![PERCEIVE, SAY]]);
            let connection = ScriptedConnection::new(vec![
                status_then_wake(),
                answer("perceive", &[lost, VIEW]),
            ]);
            let command = "perceive".to_string();
            let result = play(&model, &connection).await;
            assert_eq!(result, Err(Error::Unanswered { command }), "{lost}");
            // No retry, no commit and no further commands.
            assert_eq!(connection.commands(), ["status", "perceive"], "{lost}");
        }
    }
}
