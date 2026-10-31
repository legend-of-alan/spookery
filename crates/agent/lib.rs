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
    CommitBody, GmEvent, IntentCall, PackSummary, Queued, Refusal, Roll, SayArgs, SayEvent,
    Speaker, TableStatus,
};

/// What a client feeds the state machine: an event, or a tool's reply.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Event(GmEvent),
    StatusReply(Result<TableStatus, Refusal>),
    ViewReply(Result<protocol::View, Refusal>),
    SayReply(Result<SayEvent, Refusal>),
    QueueReply(Result<Queued, Refusal>),
    CommitReply(Result<CommitBody, Refusal>),
    ModelReply(Result<Option<ModelReply>, CompletionError>),
}

/// What the state machine asks the client to do next.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Idle,
    FetchStatus { since: Option<u64> },
    Perceive { viewer: String },
    Say(SayArgs),
    QueueIntent(IntentCall),
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
    high_water: u64,
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
    intent: Option<IntentCall>,
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
    PreCommitStatus,
    Commit,
    PostCommitStatus,
}

impl Agent {
    pub fn new() -> Self {
        Self {
            high_water: u64::MAX,
            ..Self::default()
        }
    }

    pub fn start(&self) -> Step {
        Step::FetchStatus { since: None }
    }

    /// The warning the client should print before its next step. `Option<String>`, not
    /// `World::take_warning`'s `Option<&'static str>`: the engine's warnings are fixed
    /// sentences, the agent's carry a model call's error text.
    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }

    pub fn feed(&mut self, input: Input) -> Result<Step, Error> {
        if let Input::Event(event) = input {
            self.absorb(event);
            return match self.phase {
                Phase::Idle => Ok(self.next_turn()),
                _ => Ok(Step::Idle),
            };
        }
        match (std::mem::replace(&mut self.phase, Phase::Idle), input) {
            (Phase::InitStatus, Input::StatusReply(reply)) => {
                let mut status = reply.map_err(Error::Refused)?;
                self.pack = status.pack.take().ok_or(Error::OlderServer)?;
                Ok(self.finish_init(status))
            }
            (Phase::TurnStatus, Input::StatusReply(reply)) => {
                self.turn.status = Some(reply.map_err(Error::Refused)?);
                Ok(self.advance_prepare())
            }
            (Phase::Perceive(viewer), Input::ViewReply(reply)) => {
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
            (Phase::Intent, Input::QueueReply(_)) => Ok(self.advance_actions(false)),
            (Phase::PreCommitStatus, Input::StatusReply(reply)) => {
                if reply.map_err(Error::Refused)?.queued.is_empty() {
                    Ok(self.finish_commit(false))
                } else {
                    self.phase = Phase::Commit;
                    Ok(Step::CommitRound)
                }
            }
            (Phase::Commit, Input::CommitReply(reply)) => match reply {
                Ok(commit) => {
                    self.turn.checks = commit.rolls;
                    Ok(self.finish_commit(true))
                }
                Err(_) => Ok(self.finish_commit(false)),
            },
            (Phase::PostCommitStatus, Input::StatusReply(reply)) => {
                if !reply.map_err(Error::Refused)?.queued.is_empty() {
                    self.wakes.push_back(Wake::Queued);
                }
                Ok(self.narrate_checks())
            }
            _ => Err(Error::Unexpected),
        }
    }

    fn absorb(&mut self, event: GmEvent) {
        match event {
            GmEvent::Say(say)
                if matches!(say.from, Speaker::Player(_)) && say.seq > self.high_water =>
            {
                self.wakes.push_back(Wake::Say(say));
            }
            GmEvent::Queued(_) => self.wakes.push_back(Wake::Queued),
            GmEvent::Say(_) | GmEvent::Check(_) => {}
        }
    }

    fn finish_init(&mut self, status: TableStatus) -> Step {
        self.high_water = status.last_seq;
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
        Step::FetchStatus {
            since: Some(self.high_water),
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
                    new_lines: player_lines(status, self.high_water),
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
        self.phase = Phase::PreCommitStatus;
        Step::FetchStatus { since: None }
    }

    fn finish_commit(&mut self, committed: bool) -> Step {
        let status = self.turn.status.as_ref().expect("status");
        let window = player_lines(status, self.high_water);
        if let Some(max) = window.iter().map(|line| line.seq).max() {
            self.high_water = max;
        }
        for line in window.into_iter().chain(self.turn.submitted.drain(..)) {
            keep_recent(&mut self.buffer, line);
        }
        self.wakes.retain(|wake| match wake {
            Wake::Say(say) => say.seq > self.high_water,
            Wake::Queued => !committed,
        });
        if committed {
            self.phase = Phase::PostCommitStatus;
            return Step::FetchStatus { since: None };
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

fn player_lines(status: &TableStatus, high: u64) -> Vec<TapeLine> {
    status
        .events
        .iter()
        .flatten()
        .filter(|say| matches!(say.from, Speaker::Player(_)) && say.seq > high)
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
    const SAY: &str = r#"{"kind":"say","seq":2,"from":"player:pc","address":"table","place":null,"actor":null,"recipients":[],"text":"ok"}"#;

    fn status(raw: &str) -> Input {
        Input::StatusReply(Ok(serde_json::from_str(raw).unwrap()))
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
        assert_eq!(&feed(&mut a, Input::StatusReply(Ok(first))), since);
        prepare(&mut a, QUEUED_STATUS);
        a
    }

    fn prepare(a: &mut Agent, raw: &str) {
        let perceive = Step::Perceive {
            viewer: "pc".to_string(),
        };
        assert_eq!(feed(a, status(raw)), perceive);
        let view = Input::ViewReply(Ok(serde_json::from_str(VIEW).unwrap()));
        assert!(matches!(feed(a, view), Step::CallModel(r) if !r.says_only));
    }

    #[test]
    fn gm_turns_handle_queue_says_epilogue_and_transport_warnings() {
        let st = Step::FetchStatus { since: None };
        let since = Step::FetchStatus { since: Some(1) };
        let say = Step::Say(SayArgs::Table {
            text: "ok".to_string(),
        });
        let said = || Input::SayReply(Ok(serde_json::from_str(SAY).unwrap()));
        let commit = Input::CommitReply(Ok(serde_json::from_str(COMMIT).unwrap()));

        let mut a = sit_queued(&since);
        assert_eq!(feed_reply(&mut a, "[]"), st);
        assert_eq!(feed(&mut a, status(QUEUED_STATUS)), Step::CommitRound);
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
        assert_eq!(
            feed_reply(&mut a, r#"[{"address":"table","text":"ok"}]"#),
            say
        );
        assert_eq!(feed(&mut a, said()), st);
        assert_eq!(feed(&mut a, status(EMPTY_STATUS)), Step::Idle);

        let mut a = sit_queued(&since);
        let err = Input::ModelReply(Err(CompletionError::Transport("boom".into())));
        assert_eq!(feed(&mut a, err), st);
        assert_eq!(a.take_warning().as_deref(), Some("boom"));
        assert_eq!(a.take_warning(), None);
        assert_eq!(feed(&mut a, status(QUEUED_STATUS)), Step::CommitRound);
    }

    #[test]
    fn a_first_status_without_a_pack_summary_is_an_older_server() {
        let mut a = Agent::new();
        assert_eq!(a.feed(status(EMPTY_STATUS)), Err(Error::OlderServer));
    }
}
