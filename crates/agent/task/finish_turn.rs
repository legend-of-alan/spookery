//! The `finish_turn` node.

use std::sync::Arc;

use async_trait::async_trait;
use graph_flow::{NextAction, Task, TaskResult};

use crate::turn::FINISH_TURN;
use crate::{Agent, Connection, Model};

/// Finishes the turn: commits a queued move.
pub(crate) struct FinishTurn<M, C>(pub(crate) Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for FinishTurn<M, C> {
    fn id(&self) -> &str {
        FINISH_TURN
    }

    async fn run(&self, _context: graph_flow::Context) -> graph_flow::Result<TaskResult> {
        self.0.commit_queued_moves().await?;
        Ok(TaskResult::new(None, NextAction::End))
    }
}
