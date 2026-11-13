//! The `finish_turn` node.

use std::sync::Arc;

use async_trait::async_trait;
use graph_flow::{NextAction, Task, TaskResult};
use serde_json::json;

use crate::tool::COMMIT_ROUND;
use crate::turn::{call_count, FINISH_TURN, MAX_TOOL_CALLS};
use crate::{Agent, Connection, Model};

/// Finishes the turn: warns when the step limit cut it short, and commits a queued move.
pub(crate) struct FinishTurn<M, C>(pub(crate) Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for FinishTurn<M, C> {
    fn id(&self) -> &str {
        FINISH_TURN
    }

    async fn run(&self, context: graph_flow::Context) -> graph_flow::Result<TaskResult> {
        if call_count(&context) >= MAX_TOOL_CALLS {
            (self.0.report)(&format!(
                "step limit: the GM turn stopped after {MAX_TOOL_CALLS} tool calls"
            ));
        }
        if !self.0.state().queued_moves.is_empty() {
            self.0.send_command("commit", json!(COMMIT_ROUND)).await?;
        }
        Ok(TaskResult::new(None, NextAction::End))
    }
}
