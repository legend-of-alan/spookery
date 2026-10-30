//! One model call and the request/response codecs it uses.

use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionRequest {
    pub prompt: String,
    pub says_only: bool,
}

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

/// One model call. The HTTP client and the scripted model both implement this.
pub trait Model {
    fn complete(
        &mut self,
        request: &CompletionRequest,
    ) -> impl Future<Output = Result<Option<String>, CompletionError>>;
}

#[derive(Clone, Default)]
pub struct ScriptedModel {
    inner: Arc<Mutex<ScriptedModelInner>>,
}

#[derive(Default)]
struct ScriptedModelInner {
    replies: VecDeque<Result<Option<String>, CompletionError>>,
    calls: Vec<CompletionRequest>,
}

impl ScriptedModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, reply: Result<Option<String>, CompletionError>) {
        self.inner.lock().expect("script").replies.push_back(reply);
    }

    pub fn calls(&self) -> Vec<CompletionRequest> {
        self.inner.lock().expect("script").calls.clone()
    }
}

impl Model for ScriptedModel {
    async fn complete(
        &mut self,
        request: &CompletionRequest,
    ) -> Result<Option<String>, CompletionError> {
        let mut inner = self.inner.lock().expect("script");
        inner.calls.push(request.clone());
        inner.replies.pop_front().unwrap_or(Ok(None))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Timeout,
    Connect,
    Status(u16),
    Other,
}

pub fn is_retryable(class: FailureKind) -> bool {
    match class {
        FailureKind::Timeout | FailureKind::Connect => true,
        FailureKind::Status(code) => matches!(code, 429 | 500..=599),
        FailureKind::Other => false,
    }
}

pub fn transport_error(class: FailureKind, total: Duration, detail: &str) -> CompletionError {
    let message = match class {
        FailureKind::Timeout => format!("model call timed out after {}s", total.as_secs()),
        FailureKind::Connect => "model call failed to connect".to_string(),
        FailureKind::Status(code) if detail.is_empty() => format!("model call returned {code}"),
        FailureKind::Status(code) => format!("model call returned {code}: {detail}"),
        FailureKind::Other => format!("model call failed: {detail}"),
    };
    CompletionError::Transport(message)
}

/// Longest provider error text kept on the error line, in characters.
pub const MAX_DETAIL_CHARS: usize = 200;

/// The provider's error body as one short line. The key is replaced before the
/// cut so no part of it can survive, and control characters and runs of
/// whitespace collapse to single spaces.
pub fn provider_detail(body: &str, key: &str) -> String {
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

/// Chat-completions body. The fields are `messages`, `model`, and `stream`,
/// plus one function, `act`.
pub fn chat_body(model: &str, prompt: &str, says_only: bool) -> Value {
    json!({
        "messages": [
            { "role": "user", "content": prompt }
        ],
        "model": model,
        "stream": false,
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "act",
                    "description": if says_only { "Submit says." } else { "Submit says and intents." },
                    "parameters": act_parameters(says_only)
                }
            }
        ]
    })
}

fn act_parameters(says_only: bool) -> Value {
    let say = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["address", "text"],
        "properties": {
            "address": { "type": "string", "enum": ["table", "place", "actor"] },
            "text": { "type": "string" },
            "place": { "type": "string" },
            "actor": { "type": "string" }
        }
    });
    if says_only {
        return json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["says"],
            "properties": {
                "says": { "type": "array", "items": say }
            }
        });
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["says", "intents"],
        "properties": {
            "says": { "type": "array", "items": say },
            "intents": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["actor_id", "move"],
                    "properties": {
                        "actor_id": { "type": "string" },
                        "move": { "type": "string" },
                        "target": { "type": "string" }
                    }
                }
            }
        }
    })
}

pub fn parse_completion(text: &str) -> Result<Option<String>, CompletionError> {
    let value: Value =
        serde_json::from_str(text).map_err(|err| CompletionError::InvalidReply(err.to_string()))?;
    let choices = value
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| CompletionError::InvalidReply("choices".to_string()))?;
    let Some(choice) = choices.first() else {
        return Err(CompletionError::InvalidReply("choices".to_string()));
    };
    let Some(message) = choice.get("message") else {
        return Err(CompletionError::InvalidReply("message".to_string()));
    };
    let Some(calls) = message.get("tool_calls") else {
        return Ok(None);
    };
    if calls.is_null() {
        return Ok(None);
    }
    let calls = calls
        .as_array()
        .ok_or_else(|| CompletionError::InvalidReply("tool_calls".to_string()))?;
    for call in calls {
        let function = call.get("function");
        if function
            .and_then(|item| item.get("name"))
            .and_then(Value::as_str)
            != Some("act")
        {
            continue;
        }
        let Some(arguments) = function.and_then(|item| item.get("arguments")) else {
            return Err(CompletionError::InvalidReply("arguments".to_string()));
        };
        if let Some(text) = arguments.as_str() {
            if serde_json::from_str::<Value>(text).is_err() {
                return Err(CompletionError::InvalidReply("arguments".to_string()));
            }
            return Ok(Some(text.to_string()));
        }
        if arguments.is_object() {
            return Ok(Some(arguments.to_string()));
        }
        return Err(CompletionError::InvalidReply("arguments".to_string()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_body_names_act_and_carries_only_the_prompt() {
        let body = chat_body("grok-test", "the table is quiet", false);
        let text = body.to_string();
        assert!(text.contains("\"name\":\"act\""), "{text}");
        assert!(text.contains("the table is quiet"), "{text}");
        assert!(!text.contains("CAST-BODY-SENTINEL"), "{text}");
        assert_eq!(body["stream"], false);
        assert_eq!(body["model"], "grok-test");
    }

    #[test]
    fn prose_without_a_function_call_is_empty() {
        let text = r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#;
        let parsed = parse_completion(text).unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn act_arguments_are_returned() {
        let text = r#"{"choices":[{"message":{"role":"assistant","content":"ignored","tool_calls":[{"type":"function","function":{"name":"act","arguments":"{\"says\":[],\"intents\":[]}"}}]}}]}"#;
        let parsed = parse_completion(text).unwrap().unwrap();
        assert!(parsed.contains("\"says\""));
    }

    #[test]
    fn retry_classifier_retries_timeout_connect_429_and_5xx_only() {
        assert!(is_retryable(FailureKind::Timeout));
        assert!(is_retryable(FailureKind::Connect));
        assert!(is_retryable(FailureKind::Status(429)));
        assert!(is_retryable(FailureKind::Status(500)));
        assert!(is_retryable(FailureKind::Status(503)));
        assert!(is_retryable(FailureKind::Status(599)));
        assert!(!is_retryable(FailureKind::Status(400)));
        assert!(!is_retryable(FailureKind::Status(401)));
        assert!(!is_retryable(FailureKind::Status(404)));
        assert!(!is_retryable(FailureKind::Status(418)));
        assert!(!is_retryable(FailureKind::Other));
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
    fn provider_detail_is_one_short_line_without_the_key() {
        let key = "xai-SECRETKEY123";
        let body = format!("{{\"error\":\n  \"Incorrect API key provided: {key}\"\r\n}}");
        let detail = provider_detail(&body, key);
        assert_eq!(
            detail,
            "{\"error\": \"Incorrect API key provided: [key]\" }"
        );
        let long = format!("{}{key}{}", "a".repeat(195), "b".repeat(300));
        let detail = provider_detail(&long, key);
        assert!(!detail.contains("xai-"), "{detail}");
        assert!(!detail.contains('\n'), "{detail}");
        assert_eq!(detail.chars().count(), MAX_DETAIL_CHARS + 3, "{detail}");
        assert!(detail.ends_with("..."), "{detail}");
        assert_eq!(provider_detail("", key), "");
        assert_eq!(
            match transport_error(FailureKind::Status(401), Duration::from_secs(60), "bad key") {
                CompletionError::Transport(message) => message,
                CompletionError::InvalidReply(message) => message,
            },
            "model call returned 401: bad key"
        );
    }

    fn line(class: FailureKind, secs: u64) -> String {
        let detail = match class {
            FailureKind::Status(_) => "",
            _ => "detail",
        };
        match transport_error(class, Duration::from_secs(secs), detail) {
            CompletionError::Transport(message) => message,
            CompletionError::InvalidReply(message) => message,
        }
    }
}
