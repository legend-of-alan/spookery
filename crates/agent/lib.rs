//! The Spookery AI GM state machine.

pub mod llm;
mod prompt;

pub use llm::*;
use prompt::*;
pub use spookery_protocol as protocol;

use std::collections::{BTreeMap, VecDeque};

use serde_json::{json, Value};

use crate::protocol::{CommitBody, EventBody, GmSay, PlayerMessage, Roll, SessionStatus};

#[derive(Clone, Debug, PartialEq)]
pub enum AgentInput {
    GmEvent(EventBody),
    ToolResult {
        body: Value,
        refused: bool,
        raw: String,
    },
    PlayerFrame(PlayerMessage),
    ModelReply(Result<Option<String>, CompleteError>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentStep {
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
    pack: Option<PackSummary>,
    wakes: VecDeque<Wake>,
    buffer: Vec<HandledSay>,
    turn: Turn,
}

#[derive(Default)]
struct Turn {
    wake: Option<Wake>,
    status: Option<SessionStatus>,
    views: BTreeMap<String, Result<Value, String>>,
    view: Option<(TurnView, PromptContext)>,
    says: VecDeque<OutgoingSay>,
    intent: Option<IntentDraft>,
    submitted: Vec<HandledSay>,
    checks: Vec<Roll>,
}

#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    InitStatus,
    InitPack(SessionStatus),
    Idle,
    TurnStatus,
    Perceive(String),
    Model(bool),
    Say(OutgoingSay, bool),
    TurnIntent,
    AfterStatus,
    Commit,
    PostCommitStatus,
}

impl Agent {
    pub fn new_gm() -> Self {
        Self {
            high_water: u64::MAX,
            ..Self::default()
        }
    }

    pub fn start(&self) -> AgentStep {
        tool("session_status", json!({}))
    }

    pub fn feed(&mut self, input: AgentInput) -> Result<AgentStep, String> {
        if let AgentInput::GmEvent(body) = input {
            self.absorb(body);
            return match &self.phase {
                Phase::InitPack(status) if self.pack.is_some() => {
                    let status = status.clone();
                    Ok(self.finish_init(status))
                }
                Phase::Idle => Ok(self.next_turn()),
                _ => Ok(AgentStep::Idle),
            };
        }
        match (std::mem::replace(&mut self.phase, Phase::Idle), input) {
            (Phase::InitStatus, AgentInput::ToolResult { body, refused, raw }) => {
                let status = parse_status(body, refused, raw)?;
                if self.pack.is_some() {
                    Ok(self.finish_init(status))
                } else {
                    self.phase = Phase::InitPack(status);
                    Ok(AgentStep::Idle)
                }
            }
            (Phase::TurnStatus, AgentInput::ToolResult { body, refused, raw }) => {
                self.turn.status = Some(parse_status(body, refused, raw)?);
                Ok(self.advance_prepare())
            }
            (Phase::Perceive(viewer), AgentInput::ToolResult { body, refused, .. }) => {
                let res = (!refused).then_some(body).ok_or_else(|| "code".into());
                self.turn.views.insert(viewer, res);
                Ok(self.advance_prepare())
            }
            (Phase::Model(epilogue), AgentInput::ModelReply(reply)) => {
                if let Some(kept) = self.kept_from_reply(reply, epilogue) {
                    self.turn.says = kept.says.into();
                    self.turn.intent = kept.intent;
                }
                Ok(self.advance_actions(epilogue))
            }
            (Phase::Say(say, epilogue), AgentInput::ToolResult { body, refused, raw }) => {
                if !refused {
                    let spoken: GmSay = serde_json::from_value(body).map_err(|_| raw)?;
                    let handled = HandledSay::from_outgoing(&say, Some(spoken.seq));
                    if epilogue {
                        push_handled(&mut self.buffer, handled);
                    } else {
                        self.turn.submitted.push(handled);
                    }
                }
                Ok(self.advance_actions(epilogue))
            }
            (Phase::TurnIntent, AgentInput::ToolResult { .. }) => Ok(self.advance_actions(false)),
            (Phase::AfterStatus, AgentInput::ToolResult { body, refused, raw }) => {
                if parse_status(body, refused, raw)?.queued.is_empty() {
                    Ok(self.finish_commit(false))
                } else {
                    self.phase = Phase::Commit;
                    Ok(tool("commit_round", json!({ "include_tick": true })))
                }
            }
            (Phase::Commit, AgentInput::ToolResult { body, refused, raw }) => {
                if refused {
                    Ok(self.finish_commit(false))
                } else {
                    let commit: CommitBody = serde_json::from_value(body).map_err(|_| raw)?;
                    self.turn.checks = commit.checks;
                    Ok(self.finish_commit(true))
                }
            }
            (Phase::PostCommitStatus, AgentInput::ToolResult { body, refused, raw }) => {
                if !parse_status(body, refused, raw)?.queued.is_empty() {
                    self.wakes.push_back(Wake::Queued);
                }
                Ok(self.start_checks())
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
            EventBody::Pack(pack) => self.pack = Some(pack),
            EventBody::Say(_) | EventBody::Check(_) => {}
        }
    }

    fn finish_init(&mut self, status: SessionStatus) -> AgentStep {
        self.high_water = status.last_seq;
        self.sit_seq = Some(status.last_seq);
        if !status.queued.is_empty() && !self.wakes.contains(&Wake::Queued) {
            self.wakes.push_front(Wake::Queued);
        }
        self.next_turn()
    }

    fn next_turn(&mut self) -> AgentStep {
        let Some(wake) = self.wakes.pop_front() else {
            self.phase = Phase::Idle;
            return AgentStep::Idle;
        };
        self.turn = Turn {
            wake: Some(wake),
            ..Turn::default()
        };
        self.phase = Phase::TurnStatus;
        tool("session_status", json!({ "since": self.high_water }))
    }

    fn advance_prepare(&mut self) -> AgentStep {
        let pack = self.pack.as_ref().expect("pack");
        let wake = self.turn.wake.as_ref().expect("wake");
        let status = self.turn.status.as_ref().expect("status");
        let views = &self.turn.views;
        let mut missing = None;
        let prepared = prepare_with(pack, wake, &status.presence.actors_present, &mut |viewer| {
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
            return tool("perceive", args);
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

    fn call_model(&mut self, prompt: String, says_only: bool) -> AgentStep {
        self.phase = Phase::Model(says_only);
        AgentStep::CallModel {
            request: CompletionRequest { prompt, says_only },
            warning: self.warning.take(),
        }
    }

    fn kept_from_reply(
        &mut self,
        reply: Result<Option<String>, CompleteError>,
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
        let pack = self.pack.as_ref().expect("pack");
        let scope = &self.turn.view.as_ref().expect("view").0.scope;
        parse_actions(&text, says_only).map(|actions| keep_actions(actions, scope, &pack.menu))
    }

    fn advance_actions(&mut self, epilogue: bool) -> AgentStep {
        if let Some(say) = self.turn.says.pop_front() {
            let args = say.arguments();
            self.phase = Phase::Say(say, epilogue);
            return tool("say", args);
        }
        if epilogue {
            return self.next_turn();
        }
        if let Some(intent) = self.turn.intent.take() {
            self.phase = Phase::TurnIntent;
            return tool("intent", intent.arguments());
        }
        self.phase = Phase::AfterStatus;
        tool("session_status", json!({}))
    }

    fn finish_commit(&mut self, committed: bool) -> AgentStep {
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
            return tool("session_status", json!({}));
        }
        self.start_checks()
    }

    fn start_checks(&mut self) -> AgentStep {
        if !self.turn.checks.is_empty() {
            if let Some((turn_view, context)) = &mut self.turn.view {
                context.buffer = self.buffer.clone();
                context.checks = self.turn.checks.clone();
                let pack = self.pack.as_ref().expect("pack");
                let prompt = render_prompt(&pack.menu, context, turn_view, &self.turn.checks);
                return self.call_model(prompt, true);
            }
        }
        self.next_turn()
    }
}

fn tool(name: &'static str, arguments: Value) -> AgentStep {
    AgentStep::CallTool { name, arguments }
}

fn parse_status(body: Value, refused: bool, raw: String) -> Result<SessionStatus, String> {
    if refused {
        return Err(raw);
    }
    serde_json::from_value(body).map_err(|_| raw)
}

fn player_lines(status: &SessionStatus, high: u64) -> Vec<HandledSay> {
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

    const Q_ST: &str = r#"{"presence":{"humans":1,"actors_present":["pc"]},"pause":{"named_plot_clocks":"running"},"world_time":0,"tick_index":0,"state_rev":1,"pack_rev":"p","last_tick_at":"","queued":[{"id":"q1","actor_id":"pc","move":"wait"}],"last_seq":1}"#;
    const E_ST: &str = r#"{"presence":{"humans":1,"actors_present":["pc"]},"pause":{"named_plot_clocks":"running"},"world_time":0,"tick_index":0,"state_rev":1,"pack_rev":"p","last_tick_at":"","queued":[],"last_seq":1}"#;
    const VIEW: &str = r#"{"viewer":"player:pc","place":"docks","visible_actors":["pc"]}"#;
    const COMMIT: &str = r#"{"state_rev":2,"diff_summary":[],"log_path":null,"checks":[{"faces":[6,6],"stat":1,"total":13,"target":10,"outcome":"success"}]}"#;
    const SAY: &str = r#"{"kind":"say","seq":2,"from":"player:pc","address":"table","place":null,"actor":null,"recipients":[],"text":"ok"}"#;

    fn ok(a: &mut Agent, s: &str) -> AgentStep {
        let in_msg = AgentInput::ToolResult {
            body: serde_json::from_str(s).unwrap(),
            refused: false,
            raw: String::new(),
        };
        a.feed(in_msg).unwrap()
    }

    fn model(a: &mut Agent, s: &str) -> AgentStep {
        a.feed(AgentInput::ModelReply(Ok(Some(s.into())))).unwrap()
    }

    fn sit_queued(since: &AgentStep) -> Agent {
        let mut a = Agent::new_gm();
        let p = r#"{"kind":"pack","menu":[],"kinds":{"pc":"pc_template"},"places":{},"cast":{}}"#;
        let pack = serde_json::from_str(p).unwrap();
        a.feed(AgentInput::GmEvent(pack)).unwrap();
        assert_eq!(&ok(&mut a, Q_ST), since);
        prep(&mut a, Q_ST);
        a
    }

    fn prep(a: &mut Agent, s: &str) {
        assert_eq!(ok(a, s), tool("perceive", json!({"as": "pc"})));
        assert!(matches!(ok(a, VIEW), AgentStep::CallModel { request: r, .. } if !r.says_only));
    }

    #[test]
    fn gm_turns_handle_queue_says_epilogue_and_transport_warnings() {
        let st = tool("session_status", json!({}));
        let since = tool("session_status", json!({"since": 1}));
        let cm = tool("commit_round", json!({"include_tick": true}));
        let say = tool("say", json!({"address": "table", "text": "ok"}));

        let mut a = sit_queued(&since);
        assert_eq!(model(&mut a, r#"{"says":[],"intents":[]}"#), st);
        assert_eq!(ok(&mut a, Q_ST), cm);
        assert_eq!(ok(&mut a, COMMIT), st);
        assert!(matches!(ok(&mut a, E_ST), AgentStep::CallModel { request: r, .. } if r.says_only));
        assert!(model(&mut a, r#"{"says":[{"address":"table","text":"ok"}]}"#) == say);
        assert_eq!(ok(&mut a, SAY), AgentStep::Idle);

        let ev = AgentInput::GmEvent(serde_json::from_str(SAY).unwrap());
        assert_eq!(a.feed(ev).unwrap(), since);
        prep(&mut a, E_ST);
        let full = r#"{"says":[{"address":"table","text":"ok"}],"intents":[]}"#;
        assert_eq!(model(&mut a, full), say);
        assert_eq!(ok(&mut a, SAY), st);
        assert_eq!(ok(&mut a, E_ST), AgentStep::Idle);

        let mut a = sit_queued(&since);
        let err = AgentInput::ModelReply(Err(CompleteError::Transport("boom".into())));
        assert_eq!(a.feed(err).unwrap(), st);
        assert_eq!(a.warning.as_deref(), Some("boom"));
        assert_eq!(ok(&mut a, Q_ST), cm);
    }
}
