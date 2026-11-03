//! The Spookery AI GM state machine.

mod model;
mod prompt;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

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
    CommitBody, GmEvent, PackSummary, Queued, Refusal, Roll, SayArgs, SayEvent, Speaker,
    TableStatus,
};

/// What a client feeds the state machine: an event, or a tool's reply.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Event(GmEvent),
    /// The agent's input carrying the `table_status` body back, or the table's refusal.
    TableStatusReply(Result<TableStatus, Refusal>),
    /// The agent's input carrying the `perceive` body back, or the table's refusal.
    PerceiveReply(Result<protocol::View, Refusal>),
    /// The agent's input carrying the `say` body back, or the table's refusal.
    SayReply(Result<SayEvent, Refusal>),
    /// The agent's input carrying the `queue_intent` body back, or the table's refusal.
    QueueIntentReply(Result<Queued, Refusal>),
    /// The agent's input carrying the `commit_round` body back, or the table's refusal.
    CommitRoundReply(Result<CommitBody, Refusal>),
    /// The agent's input carrying the `Model`'s answer back: the `act` arguments, `None`
    /// when the model made no `act` call, or the call's error.
    ModelReply(Result<Option<ModelReply>, CompletionError>),
}

/// What the state machine asks the client to do next.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Idle,
    TableStatus { since: Option<u64> },
    Perceive { viewer: String },
    Say(SayArgs),
    QueueIntent(protocol::IntentArgs),
    CommitRound,
    CallModel(CompletionRequest),
}

/// Why the agent cannot go on, returned from `Agent::feed` to its client. Display is the
/// line the client prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The first `table_status` has no pack summary: a server built before the summary
    /// moved onto `table_status`.
    OlderServer,
    /// The table refused a `table_status` the turn needs. Display is the refusal's body.
    Refused(Refusal),
    /// A reply to a step the agent did not ask for.
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
    since_seq: u64,
    warning: Option<String>,
    phase: Phase,
    pack: PackSummary,
    wakes: VecDeque<Wake>,
    buffer: Vec<TapeLine>,
    turn: Turn,
}

#[derive(Default)]
struct Turn {
    wake: Option<Wake>,
    status: Option<TableStatus>,
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
    InitStatus,
    Idle,
    TurnStatus,
    Perceive(String),
    Model(bool),
    Say(bool),
    Intent,
    Commit,
    PostCommitStatus,
}

impl Agent {
    pub fn new() -> Self {
        Self {
            since_seq: u64::MAX,
            ..Self::default()
        }
    }

    pub fn start(&self) -> Step {
        Step::TableStatus { since: None }
    }

    /// The warning the client should print before its next step. `Option<String>`, not
    /// `World::take_warning`'s `Option<&'static str>`: the engine's warnings are fixed
    /// sentences, the agent's carry a model call's error text.
    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }

    pub fn feed(&mut self, input: Input) -> Result<Step, Error> {
        if let Input::Event(event) = input {
            self.record_event(event);
            return match self.phase {
                Phase::Idle => Ok(self.next_turn()),
                _ => Ok(Step::Idle),
            };
        }
        match (std::mem::replace(&mut self.phase, Phase::Idle), input) {
            (Phase::InitStatus, Input::TableStatusReply(reply)) => {
                let mut status = reply.map_err(Error::Refused)?;
                self.pack = status.pack.take().ok_or(Error::OlderServer)?;
                Ok(self.finish_init(status))
            }
            (Phase::TurnStatus, Input::TableStatusReply(reply)) => {
                self.turn.status = Some(reply.map_err(Error::Refused)?);
                Ok(self.advance_prepare())
            }
            (Phase::Perceive(viewer), Input::PerceiveReply(reply)) => {
                self.turn.views.insert(viewer, reply);
                Ok(self.advance_prepare())
            }
            (Phase::Model(epilogue), Input::ModelReply(reply)) => {
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
            (Phase::Say(epilogue), Input::SayReply(reply)) => {
                if let Ok(said) = reply {
                    if epilogue {
                        keep_recent(&mut self.buffer, said.into());
                    } else {
                        self.turn.submitted.push(said.into());
                    }
                }
                Ok(self.advance_actions(epilogue))
            }
            (Phase::Intent, Input::QueueIntentReply(reply)) => {
                self.turn.intent_queued = reply.is_ok();
                Ok(self.advance_actions(false))
            }
            (Phase::Commit, Input::CommitRoundReply(reply)) => match reply {
                Ok(commit) => {
                    self.turn.checks = commit.rolls;
                    Ok(self.finish_commit(true))
                }
                Err(_) => Ok(self.finish_commit(false)),
            },
            (Phase::PostCommitStatus, Input::TableStatusReply(reply)) => {
                if !reply.map_err(Error::Refused)?.queued.is_empty() {
                    self.wakes.push_back(Wake::Queued);
                }
                Ok(self.narrate_checks())
            }
            _ => Err(Error::Unexpected),
        }
    }

    fn record_event(&mut self, event: GmEvent) {
        match event {
            GmEvent::Say(say)
                if matches!(say.from, Speaker::Player(_)) && say.seq > self.since_seq =>
            {
                self.wakes.push_back(Wake::Say(say));
            }
            GmEvent::Queued(_) => self.wakes.push_back(Wake::Queued),
            GmEvent::Say(_) | GmEvent::Check(_) => {}
        }
    }

    fn finish_init(&mut self, status: TableStatus) -> Step {
        self.since_seq = status.last_seq;
        if !status.queued.is_empty() && !self.wakes.contains(&Wake::Queued) {
            self.wakes.push_front(Wake::Queued);
        }
        self.next_turn()
    }

    fn next_turn(&mut self) -> Step {
        let Some(wake) = self.wakes.pop_front() else {
            self.phase = Phase::Idle;
            return Step::Idle;
        };
        self.turn = Turn {
            wake: Some(wake),
            ..Turn::default()
        };
        self.phase = Phase::TurnStatus;
        Step::TableStatus {
            since: Some(self.since_seq),
        }
    }

    fn advance_prepare(&mut self) -> Step {
        let pack = &self.pack;
        let wake = self.turn.wake.as_ref().expect("wake");
        let status = self.turn.status.as_ref().expect("status");
        let views = &self.turn.views;
        let mut missing = None;
        let prepared = prepare_turn(pack, wake, &status.presence.actors_present, &mut |viewer| {
            if let Some(reply) = views.get(viewer) {
                reply.as_ref().ok().cloned()
            } else {
                missing.get_or_insert_with(|| viewer.to_string());
                None
            }
        });
        if let Some(viewer) = missing {
            self.phase = Phase::Perceive(viewer.clone());
            return Step::Perceive { viewer };
        }
        match prepared {
            PreparedTurn::NoModel { says } => {
                self.turn.says = says.into();
                self.advance_actions(false)
            }
            PreparedTurn::Model(turn_view) => {
                let context = PromptContext {
                    buffer: self.buffer.clone(),
                    new_lines: player_lines(status, self.since_seq),
                    queue: status.queued.clone(),
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
            return Step::Say(say);
        }
        if epilogue {
            return self.next_turn();
        }
        if let Some(intent) = self.turn.intent.take() {
            self.phase = Phase::Intent;
            return Step::QueueIntent(intent);
        }
        // Commit only when this turn saw a queue or queued its own intent, so a move queued
        // during a say-only turn keeps its wake.
        let status = self.turn.status.as_ref().expect("status");
        if status.queued.is_empty() && !self.turn.intent_queued {
            return self.finish_commit(false);
        }
        self.phase = Phase::Commit;
        Step::CommitRound
    }

    fn finish_commit(&mut self, committed: bool) -> Step {
        let status = self.turn.status.as_ref().expect("status");
        let window = player_lines(status, self.since_seq);
        if let Some(max) = window.iter().map(|line| line.seq).max() {
            self.since_seq = max;
        }
        for line in window.into_iter().chain(self.turn.submitted.drain(..)) {
            keep_recent(&mut self.buffer, line);
        }
        self.wakes.retain(|wake| match wake {
            Wake::Say(say) => say.seq > self.since_seq,
            Wake::Queued => !committed,
        });
        if committed {
            self.phase = Phase::PostCommitStatus;
            return Step::TableStatus { since: None };
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

fn player_lines(status: &TableStatus, since_seq: u64) -> Vec<TapeLine> {
    status
        .events
        .iter()
        .flatten()
        .filter(|say| matches!(say.from, Speaker::Player(_)) && say.seq > since_seq)
        .cloned()
        .map(TapeLine::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUEUED_STATUS: &str = r#"{"presence":{"players":1,"actors_present":["pc"]},"pause":{"presence_clocks":"running"},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[{"id":"q1","actor_id":"pc","move":"wait"}],"last_seq":1}"#;
    const EMPTY_STATUS: &str = r#"{"presence":{"players":1,"actors_present":["pc"]},"pause":{"presence_clocks":"running"},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[],"last_seq":1}"#;
    const VIEW: &str = r#"{"viewer":"pc","place":"docks","visible_actors":["pc"],"known_facts":[],"notes":[],"unknown_mentions":[],"clocks":[],"items":[],"habits":[]}"#;
    const COMMIT: &str = r#"{"state_rev":2,"changed_keys":[],"journal_path":null,"rolls":[{"faces":[6,6],"stat":1,"total":13,"target":10,"outcome":"success"}]}"#;
    const QUEUED: &str = r#"{"kind":"queued","id":"q2","actor_id":"pc","move":"wait"}"#;
    const SAY: &str = r#"{"kind":"say","seq":2,"from":"player:pc","address":"table","place":null,"actor":null,"recipients":[],"text":"ok"}"#;

    fn status(raw: &str) -> Input {
        Input::TableStatusReply(Ok(serde_json::from_str(raw).unwrap()))
    }

    fn feed(a: &mut Agent, input: Input) -> Step {
        a.feed(input).unwrap()
    }

    fn feed_reply(a: &mut Agent, says: &str) -> Step {
        let says = serde_json::from_str(says).unwrap();
        let reply = ModelReply {
            says,
            intents: Vec::new(),
        };
        feed(a, Input::ModelReply(Ok(Some(reply))))
    }

    fn sit_queued(since: &Step) -> Agent {
        let mut a = Agent::new();
        let mut first: TableStatus = serde_json::from_str(QUEUED_STATUS).unwrap();
        first.pack = Some(
            serde_json::from_str(
                r#"{"menu": [], "kinds": {"pc": "pc_template"}, "places": {}, "cast": {}}"#,
            )
            .unwrap(),
        );
        assert_eq!(&feed(&mut a, Input::TableStatusReply(Ok(first))), since);
        prepare(&mut a, QUEUED_STATUS);
        a
    }

    fn prepare(a: &mut Agent, raw: &str) {
        let perceive = Step::Perceive {
            viewer: "pc".to_string(),
        };
        assert_eq!(feed(a, status(raw)), perceive);
        let view = Input::PerceiveReply(Ok(serde_json::from_str(VIEW).unwrap()));
        assert!(matches!(feed(a, view), Step::CallModel(r) if !r.says_only));
    }

    #[test]
    fn gm_turns_handle_queue_says_epilogue_and_transport_warnings() {
        let st = Step::TableStatus { since: None };
        let since = Step::TableStatus { since: Some(1) };
        let say = Step::Say(SayArgs::Table {
            text: "ok".to_string(),
        });
        let said = || Input::SayReply(Ok(serde_json::from_str(SAY).unwrap()));
        let commit = Input::CommitRoundReply(Ok(serde_json::from_str(COMMIT).unwrap()));

        let mut a = sit_queued(&since);
        assert_eq!(feed_reply(&mut a, "[]"), Step::CommitRound);
        assert_eq!(feed(&mut a, commit), st);
        assert!(matches!(feed(&mut a, status(EMPTY_STATUS)), Step::CallModel(r) if r.says_only));
        assert_eq!(
            feed_reply(&mut a, r#"[{"address":"table","text":"ok"}]"#),
            say
        );
        assert_eq!(feed(&mut a, said()), Step::Idle);

        let event = Input::Event(serde_json::from_str(SAY).unwrap());
        assert_eq!(feed(&mut a, event), since);
        prepare(&mut a, EMPTY_STATUS);
        // A move queued after the say turn's status waits for a turn of its own.
        let queued = Input::Event(serde_json::from_str(QUEUED).unwrap());
        assert_eq!(feed(&mut a, queued), Step::Idle);
        assert_eq!(
            feed_reply(&mut a, r#"[{"address":"table","text":"ok"}]"#),
            say
        );
        assert_eq!(feed(&mut a, said()), since);

        let mut a = sit_queued(&since);
        let err = Input::ModelReply(Err(CompletionError::Transport("boom".into())));
        assert_eq!(feed(&mut a, err), Step::CommitRound);
        assert_eq!(a.take_warning().as_deref(), Some("boom"));
        assert_eq!(a.take_warning(), None);
    }

    #[test]
    fn a_first_status_without_a_pack_summary_is_an_older_server() {
        let mut a = Agent::new();
        assert_eq!(a.feed(status(EMPTY_STATUS)), Err(Error::OlderServer));
    }
}
