//! One model call: the turn's messages, the tool calls a reply brings, and why a call failed.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Sent from the agent to its `Model`: one message of a turn's conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// The turn's opening prompt.
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

impl CompletionError {
    /// The error line for a model call that failed as `kind`. `total` is the call's limit.
    pub fn transport(kind: FailureKind, total: Duration, detail: &str) -> Self {
        let message = match kind {
            FailureKind::Timeout => format!("model call timed out after {}s", total.as_secs()),
            FailureKind::Connect => "model call failed to connect".to_string(),
            FailureKind::Status(code) if detail.is_empty() => format!("model call returned {code}"),
            FailureKind::Status(code) => format!("model call returned {code}: {detail}"),
            FailureKind::Other => format!("model call failed: {detail}"),
        };
        CompletionError::Transport(message)
    }
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
        messages: &[Message],
    ) -> impl Future<Output = Result<Vec<ToolCall>, CompletionError>> + Send;
}

/// How a model call failed on the transport, as the client that made it saw it. The client
/// passes it to `CompletionError::transport`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Timeout,
    Connect,
    Status(u16),
    Other,
}

impl FailureKind {
    /// A timeout, a failed connect, 429 and 5xx are worth one more attempt.
    pub fn is_retryable(self) -> bool {
        match self {
            FailureKind::Timeout | FailureKind::Connect => true,
            FailureKind::Status(code) => matches!(code, 429 | 500..=599),
            FailureKind::Other => false,
        }
    }
}

/// Longest provider error text kept on the error line, in characters.
pub const MAX_DETAIL_CHARS: usize = 200;

/// The provider's error body as one short line. The key is replaced before the
/// cut so no part of it can survive, and control characters and runs of
/// whitespace collapse to single spaces.
pub fn redact_error_body(body: &str, key: &str) -> String {
    let redacted = if key.is_empty() {
        body.to_string()
    } else {
        body.replace(key, "[key]")
    };
    let line = redacted
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if line.chars().count() <= MAX_DETAIL_CHARS {
        return line;
    }
    let mut cut: String = line.chars().take(MAX_DETAIL_CHARS).collect();
    cut.push_str("...");
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_classifier_retries_timeout_connect_429_and_5xx_only() {
        assert!(FailureKind::Timeout.is_retryable());
        assert!(FailureKind::Connect.is_retryable());
        for code in [429, 500, 503, 599] {
            assert!(FailureKind::Status(code).is_retryable(), "{code}");
        }
        for code in [400, 401, 404, 418] {
            assert!(!FailureKind::Status(code).is_retryable(), "{code}");
        }
        assert!(!FailureKind::Other.is_retryable());
        assert_eq!(
            line(FailureKind::Timeout, 60),
            "model call timed out after 60s"
        );
        assert_eq!(
            line(FailureKind::Timeout, 1),
            "model call timed out after 1s"
        );
        assert_eq!(
            line(FailureKind::Connect, 60),
            "model call failed to connect"
        );
        assert_eq!(
            line(FailureKind::Status(401), 60),
            "model call returned 401"
        );
        assert_eq!(
            line(FailureKind::Status(429), 60),
            "model call returned 429"
        );
        assert_eq!(
            line(FailureKind::Status(500), 60),
            "model call returned 500"
        );
    }

    #[test]
    fn redact_error_body_is_one_short_line_without_the_key() {
        let key = "xai-SECRETKEY123";
        let body = format!("{{\"error\":\n  \"Incorrect API key provided: {key}\"\r\n}}");
        let detail = redact_error_body(&body, key);
        assert_eq!(
            detail,
            "{\"error\": \"Incorrect API key provided: [key]\" }"
        );
        let long = format!("{}{key}{}", "a".repeat(195), "b".repeat(300));
        let detail = redact_error_body(&long, key);
        assert!(!detail.contains("xai-"), "{detail}");
        assert!(!detail.contains('\n'), "{detail}");
        assert_eq!(detail.chars().count(), MAX_DETAIL_CHARS + 3, "{detail}");
        assert!(detail.ends_with("..."), "{detail}");
        assert_eq!(redact_error_body("", key), "");
        let error = CompletionError::transport(
            FailureKind::Status(401),
            Duration::from_secs(60),
            "bad key",
        );
        assert_eq!(error.to_string(), "model call returned 401: bad key");
    }

    fn line(kind: FailureKind, secs: u64) -> String {
        let detail = match kind {
            FailureKind::Status(_) => "",
            _ => "detail",
        };
        CompletionError::transport(kind, Duration::from_secs(secs), detail).to_string()
    }
}
