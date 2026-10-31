//! One model call and the request/response codecs it uses.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use serde_json::{json, Value};

use crate::prompt::ModelReply;

/// Sent from the agent to its `Model`: the prompt, and whether only says may come back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionRequest {
    pub prompt: String,
    pub says_only: bool,
}

/// Returned from a `Model` to its client, which feeds it to `Agent` in `Input::ModelReply`:
/// why one call brought no reply. Display is the warning line the client prints.
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

/// One model call. The HTTP client and the tests' scripted model both implement this.
pub trait Model {
    fn complete(
        &mut self,
        request: &CompletionRequest,
    ) -> impl Future<Output = Result<Option<ModelReply>, CompletionError>>;
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

/// Read a chat-completions reply: the first `act` call's arguments as a `ModelReply`, with
/// no intents when `says_only`. No `act` call, or arguments that are JSON but not the act
/// shape, is `None`; arguments that are not JSON are an `InvalidReply`.
pub fn parse_completion(
    text: &str,
    says_only: bool,
) -> Result<Option<ModelReply>, CompletionError> {
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
        let invalid = || CompletionError::InvalidReply("arguments".to_string());
        let arguments = match function.and_then(|item| item.get("arguments")) {
            Some(Value::String(text)) => serde_json::from_str(text).map_err(|_| invalid())?,
            Some(arguments @ Value::Object(_)) => arguments.clone(),
            _ => return Err(invalid()),
        };
        let Ok(mut reply) = serde_json::from_value::<ModelReply>(arguments) else {
            return Ok(None);
        };
        if says_only {
            reply.intents.clear();
        }
        return Ok(Some(reply));
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
        let parsed = parse_completion(text, false).unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn act_arguments_are_typed_once() {
        let act = |arguments: &str| {
            let arguments = serde_json::to_string(arguments).unwrap();
            format!(
                r#"{{"choices":[{{"message":{{"role":"assistant","content":"ignored","tool_calls":[{{"type":"function","function":{{"name":"act","arguments":{arguments}}}}}]}}}}]}}"#
            )
        };
        let full = act(r#"{"says":[],"intents":[{"actor_id":"mira","move":"wait"}]}"#);
        let reply = parse_completion(&full, false).unwrap().unwrap();
        assert_eq!(reply.intents[0].move_name.as_deref(), Some("wait"));
        assert!(parse_completion(&full, true)
            .unwrap()
            .unwrap()
            .intents
            .is_empty());
        assert_eq!(parse_completion(&act(r#"{"nope":1}"#), false), Ok(None));
        assert!(parse_completion(&act("not json"), false).is_err());
    }

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
