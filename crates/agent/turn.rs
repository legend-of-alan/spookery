//! The GM turn graph: `ask` the model, answer each tool `call` it made, and `close` the
//! turn. The graph owns every transition inside a turn; its context holds only what the
//! edges read.

use std::sync::Arc;

use async_trait::async_trait;
use graph_flow::{Context, Graph, GraphBuilder, NextAction, Session, Task, TaskResult};
use serde_json::{Map, Value};

use crate::{ChatMessage, Model, Seat, Shared, ToolCall, Turn, MAX_TOOL_CALLS};

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
pub(crate) fn graph<M: Model, S: Seat>(shared: &Arc<Shared<M, S>>) -> Graph {
    GraphBuilder::new("gm_turn")
        .add_task(Arc::new(Ask(Arc::clone(shared))))
        .add_task(Arc::new(Call(Arc::clone(shared))))
        .add_task(Arc::new(Close(Arc::clone(shared))))
        .add_conditional_edge("ask", is_done, "close", "call")
        .add_conditional_edge("call", |c| call_count(c) >= MAX_TOOL_CALLS, "close", "ask")
        .add_conditional_edge("call", |c| !pending_calls(c).is_empty(), "call", "ask")
        .build()
        .expect("turn graph")
}

/// Plays `turn` through `graph` to its end, then hands it back to the table.
pub(crate) async fn play<M: Model, S: Seat>(
    graph: &Graph,
    shared: &Shared<M, S>,
    turn: Turn,
) -> graph_flow::Result<()> {
    *shared.turn() = turn;
    let mut session = Session::new_from_task(String::new(), "ask");
    graph.execute_session(&mut session).await?;
    let turn = std::mem::take(&mut *shared.turn());
    shared.table().finish_turn(turn);
    Ok(())
}

/// Asks the model for its next tool calls.
struct Ask<M, S>(Arc<Shared<M, S>>);

#[async_trait]
impl<M: Model, S: Seat> Task for Ask<M, S> {
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
struct Call<M, S>(Arc<Shared<M, S>>);

#[async_trait]
impl<M: Model, S: Seat> Task for Call<M, S> {
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

impl<M: Model, S: Seat> Call<M, S> {
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
        match call.name.as_str() {
            "perceive" | "say" | "queue" => self.0.relay(&call.name, args).await,
            "end_turn" if move_waiting => self.0.commit().await,
            "end_turn" => Ok("nothing is queued, so there is no round to play".to_string()),
            name => Ok(format!("refused: there is no tool named {name}")),
        }
    }
}

/// Ends the turn: warns when the step limit cut it short, and commits a waiting move.
struct Close<M, S>(Arc<Shared<M, S>>);

#[async_trait]
impl<M: Model, S: Seat> Task for Close<M, S> {
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
            self.0.commit().await?;
        }
        Ok(TaskResult::new(None, NextAction::End))
    }
}
