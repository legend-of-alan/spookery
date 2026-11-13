//! One model call: the turn's messages, the tool calls a reply brings, and why a call failed.

use std::fmt;
use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::model;

/// Sent from the agent to its `Model`: one message of a turn's conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// The turn's observation.
    User { content: String },
    /// One model reply, its tool calls echoed back as they came.
    Assistant { tool_calls: Vec<ToolCall> },
    /// The result of the tool call `tool_call_id`.
    Tool {
        tool_call_id: String,
        content: String,
    },
}

/// Returned from a `Model` to the agent, and echoed back to the model in the next call:
/// one tool call. `arguments` is the raw text the model sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Returned from a `Model` to the agent: why one call brought no reply. Display is the
/// warning line the agent reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionError {
    Transport(String),
    InvalidReply(String),
}

impl fmt::Display for CompletionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompletionError::Transport(message) | CompletionError::InvalidReply(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for CompletionError {}

/// The model: given the turn's messages, it requests the next tool calls, none when it is
/// done. The HTTP client and the tests' scripted models implement this.
pub trait Model: Send + Sync + 'static {
    fn request_tool_calls(
        &self,
        messages: &[model::Message],
    ) -> impl Future<Output = Result<Vec<ToolCall>, CompletionError>> + Send;
}
