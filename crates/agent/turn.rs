//! One GM turn and the graph that plays it: `ask_model` for tool calls, `run_tool_call` for
//! each one, and `finish_turn`. The graph owns every transition inside a turn; its context
//! holds only what the edges read.

use std::sync::Arc;

use graph_flow::{Graph, GraphBuilder, Session};

use crate::context::TapeLine;
use crate::model::Message;
use crate::protocol::ServerBody;
use crate::task::{AskModel, FinishTurn, RunToolCall};
use crate::{Agent, Connection, Model, ToolCall};

/// The most tool calls one turn makes, valid or not, `end_turn` included.
pub const MAX_TOOL_CALLS: usize = 20;

/// The graph's nodes.
pub(crate) const ASK_MODEL: &str = "ask_model";
pub(crate) const RUN_TOOL_CALL: &str = "run_tool_call";
pub(crate) const FINISH_TURN: &str = "finish_turn";

/// The calls of the model's last reply that wait for a result.
pub(crate) const PENDING_CALLS: &str = "pending_calls";
/// How many tool calls this turn made.
pub(crate) const CALL_COUNT: &str = "call_count";
/// Set when the model is done or its call failed.
pub(crate) const DONE: &str = "done";

/// One turn: the conversation with the model, and the player and GM lines it records.
#[derive(Default)]
pub(crate) struct Turn {
    pub(crate) messages: Vec<Message>,
    /// The player says this turn shows the model.
    pub(crate) player_lines: Vec<TapeLine>,
    /// The says the GM sent this turn, as the server confirmed them.
    pub(crate) gm_lines: Vec<TapeLine>,
}

impl Turn {
    /// Records the reply to one of the agent's commands: the GM's confirmed say joins
    /// `gm_lines`. Every other reply leaves the turn as it is.
    pub(crate) fn record_reply(&mut self, seq: u64, body: &ServerBody) {
        if let ServerBody::Say(say) = body {
            self.gm_lines.push(TapeLine::new(seq, say.clone()));
        }
    }
}

pub(crate) fn pending_calls(context: &graph_flow::Context) -> Vec<ToolCall> {
    context.get(PENDING_CALLS).unwrap_or_default()
}

pub(crate) fn call_count(context: &graph_flow::Context) -> usize {
    context.get(CALL_COUNT).unwrap_or_default()
}

fn is_done(context: &graph_flow::Context) -> bool {
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
