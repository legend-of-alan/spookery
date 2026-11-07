//! The GM turn graph: `ask_model` for tool calls, `run_tool_call` for each one, and
//! `finish_turn`. The graph owns every transition inside a turn; its context holds only what the
//! edges read.

use std::sync::Arc;

use async_trait::async_trait;
use graph_flow::{Context, Graph, GraphBuilder, NextAction, Session, Task, TaskResult};
use serde_json::{json, Map, Value};

use crate::prompt::render_reply;
use crate::protocol::CommitArgs;
use crate::{Agent, ChatMessage, Connection, Model, ToolCall, Turn, MAX_TOOL_CALLS};

/// `commit` with its rounds, as the `end_turn` tool and `finish_turn` send it.
const COMMIT_ROUND: CommitArgs = CommitArgs {
    include_round: true,
    session_id: None,
};

/// The graph's nodes.
const ASK_MODEL: &str = "ask_model";
const RUN_TOOL_CALL: &str = "run_tool_call";
const FINISH_TURN: &str = "finish_turn";

/// The calls of the model's last reply that wait for a result.
const CALLS: &str = "calls";
/// How many tool calls this turn made.
const USED: &str = "used";
/// Set when the model is done or its call failed.
const DONE: &str = "done";

fn pending_calls(context: &Context) -> Vec<ToolCall> {
    context.get(CALLS).unwrap_or_default()
}

fn call_count(context: &Context) -> usize {
    context.get(USED).unwrap_or_default()
}

fn is_done(context: &Context) -> bool {
    context.get(DONE).unwrap_or_default()
}

/// Built once per `run`. Edges are checked in the order added and the first match wins, so
/// from `run_tool_call` the step limit comes before the waiting calls, and `ask_model` is the
/// fallback.
pub(crate) fn build_graph<M: Model, C: Connection>(agent: &Arc<Agent<M, C>>) -> Graph {
    GraphBuilder::new("gm_turn")
        .add_task(Arc::new(AskModel(Arc::clone(agent))))
        .add_task(Arc::new(RunToolCall(Arc::clone(agent))))
        .add_task(Arc::new(FinishTurn(Arc::clone(agent))))
        .add_conditional_edge(ASK_MODEL, is_done, FINISH_TURN, RUN_TOOL_CALL)
        .add_conditional_edge(
            RUN_TOOL_CALL,
            |c| call_count(c) >= MAX_TOOL_CALLS,
            FINISH_TURN,
            ASK_MODEL,
        )
        .add_conditional_edge(
            RUN_TOOL_CALL,
            |c| !pending_calls(c).is_empty(),
            RUN_TOOL_CALL,
            ASK_MODEL,
        )
        .build()
        .expect("turn graph")
}

/// Plays `turn` through `graph` to its end, then hands it back to the client state.
pub(crate) async fn play<M: Model, C: Connection>(
    graph: &Graph,
    agent: &Agent<M, C>,
    turn: Turn,
) -> graph_flow::Result<()> {
    *agent.turn() = turn;
    let mut session = Session::new_from_task(String::new(), ASK_MODEL);
    graph.execute_session(&mut session).await?;
    let turn = std::mem::take(&mut *agent.turn());
    agent.state().record_turn(turn);
    Ok(())
}

/// Asks the model for its next tool calls.
struct AskModel<M, C>(Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for AskModel<M, C> {
    fn id(&self) -> &str {
        ASK_MODEL
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let messages = self.0.turn().messages.clone();
        match self.0.model.request_tool_calls(&messages).await {
            Ok(tool_calls) if !tool_calls.is_empty() => {
                context.set(CALLS, &tool_calls)?;
                self.0
                    .turn()
                    .messages
                    .push(ChatMessage::ModelReply { tool_calls });
            }
            Ok(_) => context.set(DONE, true)?,
            Err(err) => {
                (self.0.report)(&err.to_string());
                context.set(DONE, true)?;
            }
        }
        Ok(TaskResult::new(None, NextAction::ContinueAndExecute))
    }
}

/// Runs the first waiting tool call, through the server or locally, and adds its one
/// tool result to the conversation.
struct RunToolCall<M, C>(Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for RunToolCall<M, C> {
    fn id(&self) -> &str {
        RUN_TOOL_CALL
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let mut waiting = pending_calls(&context);
        let call = waiting.remove(0);
        context.set(CALLS, waiting)?;
        context.set(USED, call_count(&context) + 1)?;
        let content = self.fetch_tool_result(&call).await?;
        self.0.turn().messages.push(ChatMessage::ToolResult {
            tool_call_id: call.id,
            content,
        });
        Ok(TaskResult::new(None, NextAction::ContinueAndExecute))
    }
}

impl<M: Model, C: Connection> RunToolCall<M, C> {
    /// Fetches `call`'s result: the server's reply to the command it asks for, or the
    /// agent's own answer when no command goes out.
    async fn fetch_tool_result(&self, call: &ToolCall) -> graph_flow::Result<String> {
        let args = match call.arguments.trim() {
            "" => Map::new(),
            text => match serde_json::from_str(text) {
                Ok(Value::Object(args)) => args,
                _ => return Ok("refused: the arguments are not a JSON object".to_string()),
            },
        };
        let move_waiting = self.0.turn().move_waiting;
        let (command, args) = match call.name.as_str() {
            "perceive" | "say" | "queue" => (call.name.as_str(), Value::Object(args)),
            "end_turn" if move_waiting => ("commit", json!(COMMIT_ROUND)),
            "end_turn" => return Ok("nothing is queued, so there is no round to play".into()),
            name => return Ok(format!("refused: there is no tool named {name}")),
        };
        let body = self.0.send_command(command, args).await?;
        Ok(render_reply(&self.0.state().pack, body))
    }
}

/// Finishes the turn: warns when the step limit cut it short, and commits a waiting move.
struct FinishTurn<M, C>(Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for FinishTurn<M, C> {
    fn id(&self) -> &str {
        FINISH_TURN
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        if call_count(&context) >= MAX_TOOL_CALLS {
            (self.0.report)(&format!(
                "step limit: the GM turn stopped after {MAX_TOOL_CALLS} tool calls"
            ));
        }
        let move_waiting = self.0.turn().move_waiting;
        if move_waiting {
            self.0.send_command("commit", json!(COMMIT_ROUND)).await?;
        }
        Ok(TaskResult::new(None, NextAction::End))
    }
}
