//! One GM turn and the graph that plays it: `ask_model` for tool calls, `run_tool_call` for
//! each one, and `finish_turn`. The graph owns every transition inside a turn; its context
//! holds only what the edges read.

use std::sync::Arc;

use graph_flow::{Graph, GraphBuilder, Session};

use crate::model;
use crate::task::{AskModel, FinishTurn, RunToolCall};
use crate::{Agent, Connection, Model, ToolCall};

/// The most model calls one turn makes. It stops a model that never ends its turn.
pub const MAX_MODEL_CALLS: usize = 20;

/// The graph's nodes.
pub(crate) const ASK_MODEL: &str = "ask_model";
pub(crate) const RUN_TOOL_CALL: &str = "run_tool_call";
pub(crate) const FINISH_TURN: &str = "finish_turn";

/// How many model calls this turn made.
pub(crate) const MODEL_CALL_COUNT: &str = "model_call_count";
/// Set when the model is done, its call failed or the turn reached `MAX_MODEL_CALLS`.
pub(crate) const DONE: &str = "done";

/// One GM turn: it handles one turn cause, may span several model calls, and is not a Spookery round.
#[derive(Default)]
pub(crate) struct Turn {
    pub(crate) conversation: Vec<model::Message>,
}

impl Turn {
    /// The first call of the model's last reply that has no tool result yet. Each call's
    /// result follows its reply in order, so the results after the reply count the calls run.
    pub(crate) fn next_call(&self) -> Option<ToolCall> {
        self.conversation
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, message)| match message {
                model::Message::Assistant { tool_calls } => {
                    let results_after = self.conversation.len() - i - 1;
                    Some(tool_calls.get(results_after).cloned())
                }
                _ => None,
            })
            .flatten()
    }
}

fn is_done(context: &graph_flow::Context) -> bool {
    context.get(DONE).unwrap_or_default()
}

/// Built once per `run`.
pub(crate) fn build_graph<M: Model, C: Connection>(agent: &Arc<Agent<M, C>>) -> Graph {
    GraphBuilder::new("gm_turn")
        .add_task(Arc::new(AskModel(Arc::clone(agent))))
        .add_task(Arc::new(RunToolCall(Arc::clone(agent))))
        .add_task(Arc::new(FinishTurn(Arc::clone(agent))))
        .add_conditional_edge(ASK_MODEL, is_done, FINISH_TURN, RUN_TOOL_CALL)
        .add_conditional_edge(
            RUN_TOOL_CALL,
            {
                let agent = Arc::clone(agent);
                move |_: &graph_flow::Context| agent.turn().next_call().is_some()
            },
            RUN_TOOL_CALL,
            ASK_MODEL,
        )
        .build()
        .expect("turn graph")
}

/// Plays `turn` through `graph` to its end.
pub(crate) async fn play<M: Model, C: Connection>(
    graph: &Graph,
    agent: &Agent<M, C>,
    turn: Turn,
) -> graph_flow::Result<()> {
    *agent.turn() = turn;
    let mut session = Session::new_from_task(String::new(), ASK_MODEL);
    graph.execute_session(&mut session).await?;
    Ok(())
}
