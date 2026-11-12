//! The `run_tool_call` node.

use std::sync::Arc;

use async_trait::async_trait;
use graph_flow::{NextAction, Task, TaskResult};

use crate::model::Message;
use crate::tool;
use crate::turn::{call_count, pending_calls, CALL_COUNT, PENDING_CALLS, RUN_TOOL_CALL};
use crate::{Agent, Connection, Model};

/// Runs the first waiting tool call, through the server or locally, and adds its one
/// tool result to the conversation.
pub(crate) struct RunToolCall<M, C>(pub(crate) Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for RunToolCall<M, C> {
    fn id(&self) -> &str {
        RUN_TOOL_CALL
    }

    async fn run(&self, context: graph_flow::Context) -> graph_flow::Result<TaskResult> {
        let mut waiting = pending_calls(&context);
        let call = waiting.remove(0);
        context.set(PENDING_CALLS, waiting)?;
        context.set(CALL_COUNT, call_count(&context) + 1)?;
        let content = tool::call(&self.0, &call).await?;
        self.0.turn().messages.push(Message::Tool {
            tool_call_id: call.id,
            content,
        });
        Ok(TaskResult::new(None, NextAction::ContinueAndExecute))
    }
}
