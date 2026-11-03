//! The Spookery AI GM state machine. It reads the GM seat's one ordered stream and decides
//! from everything up to the last `seq` it read: it never pulls a snapshot during a turn.

mod model;
mod prompt;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use serde::Serialize;
use spookery_protocol as protocol;

pub use crate::model::{
    chat_body, parse_completion, provider_detail, CompletionError, CompletionRequest, FailureKind,
    Model, MAX_DETAIL_CHARS,
};
pub use crate::prompt::ModelReply;
use crate::prompt::{
    keep_actions, keep_recent, prepare_turn, render_prompt, PreparedTurn, PromptContext, TapeLine,
    TurnView, Wake,
};
use crate::protocol::{
    ClientMessage, CommitArgs, NoArgs, PackSummary, PerceiveArgs, QueuedMove, Refusal, Roll,
    SayArgs, ServerBody, ServerMessage, Speaker,
};

/// What a client feeds the state machine: a message from the seat, or the model's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// One message from the GM seat's stream, in order. A reply is one of them.
    Message(ServerMessage),
    /// The agent's input carrying the `Model`'s answer back: the `act` arguments, `None`
    /// when the model made no `act` call, or the call's error.
    ModelReply(Result<Option<ModelReply>, CompletionError>),
}

/// What the state machine asks the client to do next.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Feed the next message.
    Idle,
    /// Send this message, then feed messages until its reply.
    Send(ClientMessage),
    CallModel(CompletionRequest),
}

/// Why the agent cannot go on, returned from `Agent::feed` to its client. Display is the
/// line the client prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The `status` reply has no pack summary: a server built before the summary moved
    /// onto `status`.
    OlderServer,
    /// The table refused the `status` the agent sits with. Display is the refusal's body.
    Refused(Refusal),
    /// A reply or a model answer the agent did not ask for.
    Unexpected,
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
            Error::Unexpected => f.write_str("unexpected agent input"),
        }
    }
}

impl std::error::Error for Error {}

/// The AI GM's turns at one GM seat: fed `Input`s, it answers with the next `Step`.
#[derive(Default)]
pub struct Agent {
    /// The number in the next message id.
    next_id: u64,
    /// The id of the message whose reply the agent waits for.
    pending: Option<String>,
    warning: Option<String>,
    phase: Phase,
    pack: PackSummary,
    /// Who is seated, as of the last message read.
    present: Vec<String>,
    /// The queue, as of the last message read.
    queue: Vec<QueuedMove>,
    /// Player says read since a turn last took them.
    heard: Vec<TapeLine>,
    wakes: VecDeque<Wake>,
    buffer: Vec<TapeLine>,
    turn: Turn,
}

#[derive(Default)]
struct Turn {
    wake: Option<Wake>,
    /// The turn saw a queue when it started.
    queue_seen: bool,
    /// The player says this turn shows the model.
    lines: Vec<TapeLine>,
    views: BTreeMap<String, Result<protocol::View, Refusal>>,
    view: Option<(TurnView, PromptContext)>,
    says: VecDeque<SayArgs>,
    intent: Option<protocol::IntentArgs>,
    intent_queued: bool,
    submitted: Vec<TapeLine>,
    checks: Vec<Roll>,
}

#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Status,
    Idle,
    Perceive(String),
    Model(bool),
    Say(bool),
    Intent,
    Commit,
}

impl Agent {
    pub fn new() -> Self {
        Self::default()
    }

    /// The first step: ask for the status the agent sits with.
    pub fn start(&mut self) -> Step {
        self.phase = Phase::Status;
        self.send("status", &NoArgs {})
    }

    /// The warning the client should print before its next step. `Option<String>`, not
    /// `World::take_warning`'s `Option<&'static str>`: the engine's warnings are fixed
    /// sentences, the agent's carry a model call's error text.
    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }

    pub fn feed(&mut self, input: Input) -> Result<Step, Error> {
        let message = match input {
            Input::Message(message) => message,
            Input::ModelReply(reply) => return self.feed_model_reply(reply),
        };
        // Every message counts, a reply too, so the agent knows everything up to its seq.
        self.record(&message);
        let replied = message.reply_to.is_some() && message.reply_to == self.pending;
        if !replied {
            return Ok(match self.phase {
                Phase::Idle => self.next_turn(),
                _ => Step::Idle,
            });
        }
        self.pending = None;
        let seq = message.seq.unwrap_or_default();
        match (
            std::mem::replace(&mut self.phase, Phase::Idle),
            message.body,
        ) {
            (Phase::Status, ServerBody::Status(mut status)) => {
                self.pack = status.pack.take().ok_or(Error::OlderServer)?;
                self.present = status.presence.actors_present;
                self.queue = status.queued;
                if !self.queue.is_empty() && !self.wakes.contains(&Wake::Queued) {
                    self.wakes.push_front(Wake::Queued);
                }
                Ok(self.next_turn())
            }
            (Phase::Status, ServerBody::Refused(refusal)) => Err(Error::Refused(refusal)),
            (Phase::Perceive(viewer), body) => {
                let reply = match body {
                    ServerBody::View(view) => Ok(*view),
                    ServerBody::Refused(refusal) => Err(refusal),
                    _ => return Err(Error::Unexpected),
                };
                self.turn.views.insert(viewer, reply);
                Ok(self.advance_prepare())
            }
            (Phase::Say(epilogue), body) => {
                if let ServerBody::Say(said) = body {
                    let line = TapeLine::new(seq, said);
                    if epilogue {
                        keep_recent(&mut self.buffer, line);
                    } else {
                        self.turn.submitted.push(line);
                    }
                }
                Ok(self.advance_actions(epilogue))
            }
            (Phase::Intent, body) => {
                self.turn.intent_queued = matches!(body, ServerBody::Queued { .. });
                Ok(self.advance_actions(false))
            }
            (Phase::Commit, body) => {
                if let ServerBody::Committed(commit) = body {
                    self.turn.checks = commit.rolls;
                }
                Ok(self.finish_turn())
            }
            _ => Err(Error::Unexpected),
        }
    }

    fn feed_model_reply(
        &mut self,
        reply: Result<Option<ModelReply>, CompletionError>,
    ) -> Result<Step, Error> {
        let Phase::Model(epilogue) = std::mem::replace(&mut self.phase, Phase::Idle) else {
            return Err(Error::Unexpected);
        };
        match reply {
            Err(err) => self.warning = Some(err.to_string()),
            Ok(None) => {}
            Ok(Some(reply)) => {
                let scope = &self.turn.view.as_ref().expect("view").0.scope;
                let (says, intent) = keep_actions(reply, scope, &self.pack.menu);
                self.turn.says = says.into();
                self.turn.intent = intent;
            }
        }
        Ok(self.advance_actions(epilogue))
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

    /// Asks the client to send `word`, and waits for its reply.
    fn send(&mut self, word: &str, args: &impl Serialize) -> Step {
        self.next_id += 1;
        let id = format!("a{}", self.next_id);
        let args = serde_json::to_value(args).expect("word arguments");
        let message = ClientMessage::new(&id, word, args);
        self.pending = Some(id);
        Step::Send(message)
    }

    fn next_turn(&mut self) -> Step {
        let Some(wake) = self.wakes.pop_front() else {
            self.phase = Phase::Idle;
            return Step::Idle;
        };
        self.turn = Turn {
            wake: Some(wake),
            queue_seen: !self.queue.is_empty(),
            lines: std::mem::take(&mut self.heard),
            ..Turn::default()
        };
        self.advance_prepare()
    }

    fn advance_prepare(&mut self) -> Step {
        let pack = &self.pack;
        let wake = self.turn.wake.as_ref().expect("wake");
        let views = &self.turn.views;
        let mut missing = None;
        let prepared = prepare_turn(pack, wake, &self.present, &mut |viewer| {
            if let Some(reply) = views.get(viewer) {
                reply.as_ref().ok().cloned()
            } else {
                missing.get_or_insert_with(|| viewer.to_string());
                None
            }
        });
        if let Some(viewer) = missing {
            self.phase = Phase::Perceive(viewer.clone());
            let args = PerceiveArgs {
                as_viewer: Some(viewer),
                max_body_chars: None,
            };
            return self.send("perceive", &args);
        }
        match prepared {
            PreparedTurn::NoModel { says } => {
                self.turn.says = says.into();
                self.advance_actions(false)
            }
            PreparedTurn::Model(turn_view) => {
                let context = PromptContext {
                    buffer: self.buffer.clone(),
                    new_lines: self.turn.lines.clone(),
                    queue: self.queue.clone(),
                    checks: Vec::new(),
                };
                let prompt = render_prompt(&pack.menu, &context, &turn_view, &[]);
                self.turn.view = Some((*turn_view, context));
                self.call_model(prompt, false)
            }
        }
    }

    fn call_model(&mut self, prompt: String, says_only: bool) -> Step {
        self.phase = Phase::Model(says_only);
        Step::CallModel(CompletionRequest { prompt, says_only })
    }

    fn advance_actions(&mut self, epilogue: bool) -> Step {
        if let Some(say) = self.turn.says.pop_front() {
            self.phase = Phase::Say(epilogue);
            return self.send("say", &say);
        }
        if epilogue {
            return self.next_turn();
        }
        if let Some(intent) = self.turn.intent.take() {
            self.phase = Phase::Intent;
            return self.send("queue", &intent);
        }
        // Commit only when this turn saw a queue or queued its own intent, so a move queued
        // during a say-only turn keeps its wake.
        if !self.turn.queue_seen && !self.turn.intent_queued {
            return self.finish_turn();
        }
        self.phase = Phase::Commit;
        let args = CommitArgs {
            include_round: true,
            session_id: None,
        };
        self.send("commit", &args)
    }

    /// The turn's lines join the buffer, and a say the model has now read wakes no turn.
    fn finish_turn(&mut self) -> Step {
        let seen = self.turn.lines.iter().map(|line| line.seq).max();
        for line in self
            .turn
            .lines
            .drain(..)
            .chain(self.turn.submitted.drain(..))
        {
            keep_recent(&mut self.buffer, line);
        }
        if let Some(seen) = seen {
            self.wakes.retain(|wake| match wake {
                Wake::Say(line) => line.seq > seen,
                Wake::Queued => true,
            });
        }
        self.narrate_checks()
    }

    fn narrate_checks(&mut self) -> Step {
        if !self.turn.checks.is_empty() {
            if let Some((turn_view, context)) = &mut self.turn.view {
                context.buffer = self.buffer.clone();
                context.checks = self.turn.checks.clone();
                let prompt = render_prompt(&self.pack.menu, context, turn_view, &self.turn.checks);
                return self.call_model(prompt, true);
            }
        }
        self.next_turn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUEUED_STATUS: &str = r#"{"kind":"status","presence":{"players":1,"actors_present":["pc"]},"pause":{"presence_clocks":"running"},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[{"id":"q1","actor_id":"pc","move":"wait"}],"pack":{"menu":[],"kinds":{"pc":"pc_template"},"places":{},"cast":{}}}"#;
    const EMPTY_STATUS: &str = r#"{"kind":"status","presence":{"players":1,"actors_present":["pc"]},"pause":{"presence_clocks":"running"},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[],"pack":{"menu":[],"kinds":{"pc":"pc_template"},"places":{},"cast":{}}}"#;
    const OLD_STATUS: &str = r#"{"kind":"status","presence":{"players":1,"actors_present":["pc"]},"pause":{"presence_clocks":"running"},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[]}"#;
    const VIEW: &str = r#"{"kind":"view","viewer":"pc","place":"docks","visible_actors":["pc"],"known_facts":[],"notes":[],"unknown_mentions":[],"clocks":[],"items":[],"habits":[]}"#;
    const COMMITTED: &str = r#"{"kind":"committed","state_rev":2,"changed_keys":[],"journal_path":null,"rolls":[{"faces":[6,6],"stat":1,"total":13,"target":10,"outcome":"success"}]}"#;
    const ROUND: &str = r#"{"kind":"round","round_index":1,"world_time":5}"#;
    const QUEUED: &str = r#"{"kind":"queued","intent":"q2","actor":"pc","move":"wait"}"#;
    const PLAYER_SAY: &str = r#"{"kind":"say","from":"player:pc","address":"table","text":"ok"}"#;
    const GM_SAY: &str = r#"{"kind":"say","from":"gm","address":"table","text":"ok"}"#;

    /// One message: `body` with `seq`, answering `reply_to` when it is set.
    fn message(seq: u64, reply_to: Option<&str>, body: &str) -> Input {
        Input::Message(ServerMessage {
            id: None,
            seq: Some(seq),
            reply_to: reply_to.map(str::to_string),
            body: serde_json::from_str(body).unwrap(),
        })
    }

    fn feed(a: &mut Agent, input: Input) -> Step {
        a.feed(input).unwrap()
    }

    /// The word and id of a `Send` step.
    fn sent(step: &Step) -> (String, String) {
        let Step::Send(message) = step else {
            panic!("not a send: {step:?}");
        };
        (message.body.word.clone(), message.id.clone().unwrap())
    }

    fn feed_reply(a: &mut Agent, says: &str) -> Step {
        let says = serde_json::from_str(says).unwrap();
        let reply = ModelReply {
            says,
            intents: Vec::new(),
        };
        feed(a, Input::ModelReply(Ok(Some(reply))))
    }

    /// Sits on `status`, then answers the turn's perceive. The model is asked next.
    fn sit(status: &str) -> Agent {
        let mut a = Agent::new();
        let (word, id) = sent(&a.start());
        assert_eq!(word, "status");
        let step = feed(&mut a, message(1, Some(&id), status));
        if status == EMPTY_STATUS {
            assert_eq!(step, Step::Idle);
        } else {
            perceive(&mut a, &step, 2);
        }
        a
    }

    fn perceive(a: &mut Agent, step: &Step, seq: u64) {
        let (word, id) = sent(step);
        assert_eq!(word, "perceive");
        let step = feed(a, message(seq, Some(&id), VIEW));
        assert!(matches!(step, Step::CallModel(r) if !r.says_only));
    }

    #[test]
    fn a_queued_turn_commits_then_narrates_its_checks() {
        let mut a = sit(QUEUED_STATUS);
        let (word, id) = sent(&feed_reply(&mut a, "[]"));
        assert_eq!(word, "commit");
        // The round comes before the reply, and empties the queue.
        assert_eq!(feed(&mut a, message(3, None, ROUND)), Step::Idle);
        let step = feed(&mut a, message(4, Some(&id), COMMITTED));
        assert!(matches!(step, Step::CallModel(r) if r.says_only));
        let (word, id) = sent(&feed_reply(&mut a, r#"[{"address":"table","text":"ok"}]"#));
        assert_eq!(word, "say");
        assert_eq!(feed(&mut a, message(5, Some(&id), GM_SAY)), Step::Idle);
    }

    #[test]
    fn a_move_queued_during_a_say_turn_waits_for_its_own_turn() {
        let mut a = sit(EMPTY_STATUS);
        let step = feed(&mut a, message(2, None, PLAYER_SAY));
        perceive(&mut a, &step, 3);
        // Read mid-turn, so this turn's model never saw it.
        assert_eq!(feed(&mut a, message(4, None, QUEUED)), Step::Idle);
        let (word, id) = sent(&feed_reply(&mut a, r#"[{"address":"table","text":"ok"}]"#));
        assert_eq!(word, "say");
        // The say turn does not commit. The queued move starts the next turn.
        let step = feed(&mut a, message(5, Some(&id), GM_SAY));
        perceive(&mut a, &step, 6);
        let (word, _) = sent(&feed_reply(&mut a, "[]"));
        assert_eq!(word, "commit");
    }

    #[test]
    fn a_round_the_agent_did_not_play_drops_the_queued_wake() {
        let mut a = sit(EMPTY_STATUS);
        let step = feed(&mut a, message(2, None, PLAYER_SAY));
        perceive(&mut a, &step, 3);
        assert_eq!(feed(&mut a, message(4, None, QUEUED)), Step::Idle);
        assert_eq!(feed(&mut a, message(5, None, ROUND)), Step::Idle);
        let (word, id) = sent(&feed_reply(&mut a, r#"[{"address":"table","text":"ok"}]"#));
        assert_eq!(word, "say");
        // Someone else's round played that move, so no turn waits for it.
        assert_eq!(feed(&mut a, message(6, Some(&id), GM_SAY)), Step::Idle);
    }

    #[test]
    fn a_model_error_is_a_warning_and_the_turn_goes_on() {
        let mut a = sit(QUEUED_STATUS);
        let err = Input::ModelReply(Err(CompletionError::Transport("boom".into())));
        let (word, _) = sent(&feed(&mut a, err));
        assert_eq!(word, "commit");
        assert_eq!(a.take_warning().as_deref(), Some("boom"));
        assert_eq!(a.take_warning(), None);
    }

    #[test]
    fn a_status_without_a_pack_summary_is_an_older_server() {
        let mut a = Agent::new();
        let (_, id) = sent(&a.start());
        assert_eq!(
            a.feed(message(1, Some(&id), OLD_STATUS)),
            Err(Error::OlderServer)
        );
    }
}
