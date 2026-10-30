//! The Spookery AI GM state machine.

pub mod model;
mod prompt;

pub use model::*;
use prompt::*;
use spookery_protocol as protocol;

use std::collections::{BTreeMap, VecDeque};

use serde_json::{json, Value};

use crate::protocol::{CommitBody, EventBody, PlayerFrame, Roll, SayEvent, TableStatus};

/// A server built before the pack summary moved onto `table_status`.
const OLDER_SERVER: &str = "this spook run is older than spook ai-gm, rebuild both";

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    GmEvent(EventBody),
    ToolResult {
        body: Value,
        refused: bool,
        raw: String,
    },
    PlayerFrame(PlayerFrame),
    ModelReply(Result<Option<String>, CompletionError>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Idle,
    CallTool {
        name: &'static str,
        arguments: Value,
    },
    SendPlayerLine(String),
    CallModel {
        request: CompletionRequest,
        warning: Option<String>,
    },
}

#[derive(Default)]
pub struct Agent {
    pub sit_seq: Option<u64>,
    pub high_water: u64,
    pub warning: Option<String>,
    phase: Phase,
    pack: PackSummary,
    wakes: VecDeque<Wake>,
    buffer: Vec<HandledSay>,
    turn: Turn,
}

#[derive(Default)]
struct Turn {
    wake: Option<Wake>,
    status: Option<TableStatus>,
    views: BTreeMap<String, Result<Value, String>>,
    view: Option<(TurnView, PromptContext)>,
    says: VecDeque<SayArgs>,
    intent: Option<Intent>,
    submitted: Vec<HandledSay>,
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
    Say(SayArgs, bool),
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
        call_tool("table_status", json!({}))
    }

    pub fn feed(&mut self, input: Input) -> Result<Step, String> {
        if let Input::GmEvent(body) = input {
            self.absorb(body);
            return match self.phase {
                Phase::Idle => Ok(self.next_turn()),
                _ => Ok(Step::Idle),
            };
        }
        match (std::mem::replace(&mut self.phase, Phase::Idle), input) {
            (Phase::InitStatus, Input::ToolResult { body, refused, raw }) => {
                let mut status = parse_status(body, refused, raw)?;
                self.pack = status.pack.take().ok_or(OLDER_SERVER)?;
                Ok(self.finish_init(status))
            }
            (Phase::TurnStatus, Input::ToolResult { body, refused, raw }) => {
                self.turn.status = Some(parse_status(body, refused, raw)?);
                Ok(self.advance_prepare())
            }
            (Phase::Perceive(viewer), Input::ToolResult { body, refused, .. }) => {
                let res = (!refused).then_some(body).ok_or_else(|| "code".into());
                self.turn.views.insert(viewer, res);
                Ok(self.advance_prepare())
            }
            (Phase::Model(epilogue), Input::ModelReply(reply)) => {
                if let Some(kept) = self.parse_reply(reply, epilogue) {
                    self.turn.says = kept.says.into();
                    self.turn.intent = kept.intent;
                }
                Ok(self.advance_actions(epilogue))
            }
            (Phase::Say(say, epilogue), Input::ToolResult { body, refused, raw }) => {
                if !refused {
                    let spoken: SayEvent = serde_json::from_value(body).map_err(|_| raw)?;
                    let handled = HandledSay::from_outgoing(&say, Some(spoken.seq));
                    if epilogue {
                        push_handled(&mut self.buffer, handled);
                    } else {
                        self.turn.submitted.push(handled);
                    }
                }
                Ok(self.advance_actions(epilogue))
            }
            (Phase::Intent, Input::ToolResult { .. }) => Ok(self.advance_actions(false)),
            (Phase::PreCommitStatus, Input::ToolResult { body, refused, raw }) => {
                if parse_status(body, refused, raw)?.queued.is_empty() {
                    Ok(self.finish_commit(false))
                } else {
                    self.phase = Phase::Commit;
                    Ok(call_tool("commit_round", json!({ "include_round": true })))
                }
            }
            (Phase::Commit, Input::ToolResult { body, refused, raw }) => {
                if refused {
                    Ok(self.finish_commit(false))
                } else {
                    let commit: CommitBody = serde_json::from_value(body).map_err(|_| raw)?;
                    self.turn.checks = commit.rolls;
                    Ok(self.finish_commit(true))
                }
            }
            (Phase::PostCommitStatus, Input::ToolResult { body, refused, raw }) => {
                if !parse_status(body, refused, raw)?.queued.is_empty() {
                    self.wakes.push_back(Wake::Queued);
                }
                Ok(self.narrate_checks())
            }
            _ => Err("unexpected agent input".into()),
        }
    }

    fn absorb(&mut self, body: EventBody) {
        match body {
            EventBody::Say(say) if say.from.starts_with("player:") && say.seq > self.high_water => {
                self.wakes.push_back(Wake::Say {
                    from: say.from,
                    seq: say.seq,
                    address: say.address,
                    place: say.place,
                    actor: say.actor,
                    text: say.text,
                });
            }
            EventBody::Queued(_) => self.wakes.push_back(Wake::Queued),
            EventBody::Say(_) | EventBody::Check(_) => {}
        }
    }

    fn finish_init(&mut self, status: TableStatus) -> Step {
        self.high_water = status.last_seq;
        self.sit_seq = Some(status.last_seq);
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
        call_tool("table_status", json!({ "since": self.high_water }))
    }

    fn advance_prepare(&mut self) -> Step {
        let pack = &self.pack;
        let wake = self.turn.wake.as_ref().expect("wake");
        let status = self.turn.status.as_ref().expect("status");
        let views = &self.turn.views;
        let mut missing = None;
        let prepared = prepare_turn(pack, wake, &status.presence.actors_present, &mut |viewer| {
            if let Some(res) = views.get(viewer) {
                res.clone()
            } else {
                missing.get_or_insert_with(|| viewer.to_string());
                Err(String::new())
            }
        });
        if let Some(viewer) = missing {
            let args = json!({ "as": viewer });
            self.phase = Phase::Perceive(viewer);
            return call_tool("perceive", args);
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
                self.turn.view = Some((turn_view, context));
                self.call_model(prompt, false)
            }
        }
    }

    fn call_model(&mut self, prompt: String, says_only: bool) -> Step {
        self.phase = Phase::Model(says_only);
        Step::CallModel {
            request: CompletionRequest { prompt, says_only },
            warning: self.warning.take(),
        }
    }

    fn parse_reply(
        &mut self,
        reply: Result<Option<String>, CompletionError>,
        says_only: bool,
    ) -> Option<KeptActions> {
        let text = match reply {
            Err(err) => {
                self.warning = Some(err.to_string());
                return None;
            }
            Ok(None) => return None,
            Ok(Some(text)) => text,
        };
        if serde_json::from_str::<Value>(&text).is_err() {
            self.warning = Some("empty parse".into());
            return None;
        }
        let scope = &self.turn.view.as_ref().expect("view").0.scope;
        parse_actions(&text, says_only).map(|actions| keep_actions(actions, scope, &self.pack.menu))
    }

    fn advance_actions(&mut self, epilogue: bool) -> Step {
        if let Some(say) = self.turn.says.pop_front() {
            let args = say.to_value();
            self.phase = Phase::Say(say, epilogue);
            return call_tool("say", args);
        }
        if epilogue {
            return self.next_turn();
        }
        if let Some(intent) = self.turn.intent.take() {
            self.phase = Phase::Intent;
            return call_tool("queue_intent", intent.to_value());
        }
        self.phase = Phase::PreCommitStatus;
        call_tool("table_status", json!({}))
    }

    fn finish_commit(&mut self, committed: bool) -> Step {
        let status = self.turn.status.as_ref().expect("status");
        let window = player_lines(status, self.high_water);
        if let Some(max) = window.iter().filter_map(|say| say.seq).max() {
            self.high_water = max;
        }
        for say in window.into_iter().chain(self.turn.submitted.drain(..)) {
            push_handled(&mut self.buffer, say);
        }
        self.wakes.retain(|wake| match wake {
            Wake::Say { seq, .. } => *seq > self.high_water,
            Wake::Queued => !committed,
        });
        if committed {
            self.phase = Phase::PostCommitStatus;
            return call_tool("table_status", json!({}));
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

fn call_tool(name: &'static str, arguments: Value) -> Step {
    Step::CallTool { name, arguments }
}

fn parse_status(body: Value, refused: bool, raw: String) -> Result<TableStatus, String> {
    if refused {
        return Err(raw);
    }
    serde_json::from_value(body).map_err(|_| raw)
}

fn player_lines(status: &TableStatus, high: u64) -> Vec<HandledSay> {
    status
        .events
        .iter()
        .flatten()
        .filter(|say| say.from.starts_with("player:") && say.seq > high)
        .cloned()
        .map(HandledSay::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUEUED_STATUS: &str = r#"{"presence":{"humans":1,"actors_present":["pc"]},"pause":{"named_plot_clocks":"running"},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[{"id":"q1","actor_id":"pc","move":"wait"}],"last_seq":1}"#;
    const EMPTY_STATUS: &str = r#"{"presence":{"humans":1,"actors_present":["pc"]},"pause":{"named_plot_clocks":"running"},"world_time":0,"round_index":0,"state_rev":1,"pack_rev":"p","last_round_at":"","queued":[],"last_seq":1}"#;
    const VIEW: &str = r#"{"viewer":"pc","place":"docks","visible_actors":["pc"]}"#;
    const COMMIT: &str = r#"{"state_rev":2,"changed_keys":[],"journal_path":null,"rolls":[{"faces":[6,6],"stat":1,"total":13,"target":10,"outcome":"success"}]}"#;
    const SAY: &str = r#"{"kind":"say","seq":2,"from":"player:pc","address":"table","place":null,"actor":null,"recipients":[],"text":"ok"}"#;

    fn feed_result(a: &mut Agent, s: &str) -> Step {
        let in_msg = Input::ToolResult {
            body: serde_json::from_str(s).unwrap(),
            refused: false,
            raw: String::new(),
        };
        a.feed(in_msg).unwrap()
    }

    fn feed_reply(a: &mut Agent, s: &str) -> Step {
        a.feed(Input::ModelReply(Ok(Some(s.into())))).unwrap()
    }

    fn sit_queued(since: &Step) -> Agent {
        let mut a = Agent::new();
        let mut first: Value = serde_json::from_str(QUEUED_STATUS).unwrap();
        first["pack"] =
            json!({"menu": [], "kinds": {"pc": "pc_template"}, "places": {}, "cast": {}});
        assert_eq!(&feed_result(&mut a, &first.to_string()), since);
        prepare(&mut a, QUEUED_STATUS);
        a
    }

    fn prepare(a: &mut Agent, s: &str) {
        assert_eq!(
            feed_result(a, s),
            call_tool("perceive", json!({"as": "pc"}))
        );
        assert!(matches!(feed_result(a, VIEW), Step::CallModel { request: r, .. } if !r.says_only));
    }

    #[test]
    fn gm_turns_handle_queue_says_epilogue_and_transport_warnings() {
        let st = call_tool("table_status", json!({}));
        let since = call_tool("table_status", json!({"since": 1}));
        let cm = call_tool("commit_round", json!({"include_round": true}));
        let say = call_tool("say", json!({"address": "table", "text": "ok"}));

        let mut a = sit_queued(&since);
        assert_eq!(feed_reply(&mut a, r#"{"says":[],"intents":[]}"#), st);
        assert_eq!(feed_result(&mut a, QUEUED_STATUS), cm);
        assert_eq!(feed_result(&mut a, COMMIT), st);
        assert!(
            matches!(feed_result(&mut a, EMPTY_STATUS), Step::CallModel { request: r, .. } if r.says_only)
        );
        assert!(feed_reply(&mut a, r#"{"says":[{"address":"table","text":"ok"}]}"#) == say);
        assert_eq!(feed_result(&mut a, SAY), Step::Idle);

        let ev = Input::GmEvent(serde_json::from_str(SAY).unwrap());
        assert_eq!(a.feed(ev).unwrap(), since);
        prepare(&mut a, EMPTY_STATUS);
        let full = r#"{"says":[{"address":"table","text":"ok"}],"intents":[]}"#;
        assert_eq!(feed_reply(&mut a, full), say);
        assert_eq!(feed_result(&mut a, SAY), st);
        assert_eq!(feed_result(&mut a, EMPTY_STATUS), Step::Idle);

        let mut a = sit_queued(&since);
        let err = Input::ModelReply(Err(CompletionError::Transport("boom".into())));
        assert_eq!(a.feed(err).unwrap(), st);
        assert_eq!(a.warning.as_deref(), Some("boom"));
        assert_eq!(feed_result(&mut a, QUEUED_STATUS), cm);
    }

    #[test]
    fn a_first_status_without_a_pack_summary_is_an_older_server() {
        let mut a = Agent::new();
        let first = Input::ToolResult {
            body: serde_json::from_str(EMPTY_STATUS).unwrap(),
            refused: false,
            raw: String::new(),
        };
        assert_eq!(a.feed(first), Err(OLDER_SERVER.to_string()));
        assert_eq!(a.sit_seq, None);
    }
}
