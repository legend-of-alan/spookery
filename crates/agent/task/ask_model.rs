//! The `ask_model` node.

use std::sync::Arc;

use async_trait::async_trait;
use graph_flow::{NextAction, Task, TaskResult};

use crate::model::Message;
use crate::turn::{ASK_MODEL, DONE, MAX_MODEL_CALLS, MODEL_CALL_COUNT};
use crate::{Agent, Connection, Model};

/// Asks the model for its next tool calls, or ends the turn at `MAX_MODEL_CALLS`.
pub(crate) struct AskModel<M, C>(pub(crate) Arc<Agent<M, C>>);

#[async_trait]
impl<M: Model, C: Connection> Task for AskModel<M, C> {
    fn id(&self) -> &str {
        ASK_MODEL
    }

    async fn run(&self, context: graph_flow::Context) -> graph_flow::Result<TaskResult> {
        let count: usize = context.get(MODEL_CALL_COUNT).unwrap_or_default();
        if count >= MAX_MODEL_CALLS {
            (self.0.report)(&format!(
                "model call limit: the GM turn stopped after {MAX_MODEL_CALLS} model calls"
            ));
            context.set(DONE, true)?;
            return Ok(TaskResult::new(None, NextAction::ContinueAndExecute));
        }
        context.set(MODEL_CALL_COUNT, count + 1)?;
        let messages = self.0.turn().messages.clone();
        match self.0.model.request_tool_calls(&messages).await {
            Ok(tool_calls) if !tool_calls.is_empty() => {
                self.0
                    .turn()
                    .messages
                    .push(Message::Assistant { tool_calls });
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
