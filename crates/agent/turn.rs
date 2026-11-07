//! The GM turn graph: `ask` the model, answer each tool `call` it made, and `close` the
//! turn. The graph owns every transition inside a turn; its context holds only what the
//! edges read.

use std::sync::Arc;

use async_trait::async_trait;
use graph_flow::{Context, Graph, GraphBuilder, NextAction, Session, Task, TaskResult};
use serde_json::{json, Map, Value};

use crate::prompt::render_reply;
use crate::protocol::CommitArgs;
use crate::{Agent, ChatMessage, Connection, Model, ToolCall, Turn, MAX_TOOL_CALLS};

/// `commit` with its rounds, as `end_turn` and `close` send it.
const COMMIT_ROUND: CommitArgs = CommitArgs {
    include_round: true,
    session_id: None,
};

/// The calls of the model's last reply that wait for an answer.
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
/// from `call` the step limit comes before the waiting calls, and `ask` is the fallback.
pub(crate) fn build_graph<M: Model, C: Connection>(agent: &Arc<Agent<M, C>>) -> Graph {
    GraphBuilder::new("gm_turn")
        .add_task(Arc::new(Ask(Arc::clone(agent))))
        .add_task(Arc::new(Call(Arc::clone(agent))))
        .add_task(Arc::new(Close(Arc::clone(agent))))
        .add_conditional_edge("ask", is_done, "close", "call")
        .add_conditional_edge("call", |c| call_count(c) >= MAX_TOOL_CALLS, "close", "ask")
        .add_conditional_edge("call", |c| !pending_calls(c).is_empty(), "call", "ask")
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
    let mut session = Session::new_from_task(String::new(), "ask");
    graph.execute_session(&mut session).await?;
    let turn = std::mem::take(&mut *agent.turn());
    agent.state().finish_turn(turn);
    Ok(())
}

/// Asks the model for its next tool calls.
struct Ask<M, C>(Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for Ask<M, C> {
    fn id(&self) -> &str {
        "ask"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let messages = self.0.turn().messages.clone();
        match self.0.model.complete(&messages).await {
            Ok(tool_calls) if !tool_calls.is_empty() => {
                context.set(CALLS, &tool_calls)?;
                self.0
                    .turn()
                    .messages
                    .push(ChatMessage::Assistant { tool_calls });
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

/// Answers the first waiting call, through the seat or locally, with one tool message.
struct Call<M, C>(Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for Call<M, C> {
    fn id(&self) -> &str {
        "call"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let mut waiting = pending_calls(&context);
        let call = waiting.remove(0);
        context.set(CALLS, waiting)?;
        context.set(USED, call_count(&context) + 1)?;
        let content = self.answer(&call).await?;
        self.0.turn().messages.push(ChatMessage::Tool {
            tool_call_id: call.id,
            content,
        });
        Ok(TaskResult::new(None, NextAction::ContinueAndExecute))
    }
}

impl<M: Model, C: Connection> Call<M, C> {
    /// The table's reply to `call`, or the agent's own answer when no word goes out.
    async fn answer(&self, call: &ToolCall) -> graph_flow::Result<String> {
        let args = match call.arguments.trim() {
            "" => Map::new(),
            text => match serde_json::from_str(text) {
                Ok(Value::Object(args)) => args,
                _ => return Ok("refused: the arguments are not a JSON object".to_string()),
            },
        };
        let move_waiting = self.0.turn().move_waiting;
        let (word, args) = match call.name.as_str() {
            "perceive" | "say" | "queue" => (call.name.as_str(), Value::Object(args)),
            "end_turn" if move_waiting => ("commit", json!(COMMIT_ROUND)),
            "end_turn" => return Ok("nothing is queued, so there is no round to play".into()),
            name => return Ok(format!("refused: there is no tool named {name}")),
        };
        let body = self.0.call_word(word, args).await?;
        Ok(render_reply(&self.0.state().pack, body))
    }
}

/// Ends the turn: warns when the step limit cut it short, and commits a waiting move.
struct Close<M, C>(Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for Close<M, C> {
    fn id(&self) -> &str {
        "close"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        if call_count(&context) >= MAX_TOOL_CALLS {
            (self.0.report)(&format!(
                "step limit: the GM turn stopped after {MAX_TOOL_CALLS} tool calls"
            ));
        }
        let move_waiting = self.0.turn().move_waiting;
        if move_waiting {
            self.0.call_word("commit", json!(COMMIT_ROUND)).await?;
        }
        Ok(TaskResult::new(None, NextAction::End))
    }
}
