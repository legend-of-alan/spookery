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
pub enum CompleteError {
    Transport(String),
    EmptyParse(String),
}

impl fmt::Display for CompleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompleteError::Transport(message) | CompleteError::EmptyParse(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for CompleteError {}

/// One model call. The HTTP client and the scripted model both implement this.
pub trait Model {
    fn complete(
        &mut self,
        request: &CompletionRequest,
    ) -> impl Future<Output = Result<Option<String>, CompleteError>>;
}

#[derive(Clone, Default)]
pub struct ScriptedModel {
    inner: Arc<Mutex<ScriptedModelInner>>,
}

#[derive(Default)]
struct ScriptedModelInner {
    replies: VecDeque<Result<Option<String>, CompleteError>>,
    calls: Vec<CompletionRequest>,
}

impl ScriptedModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, reply: Result<Option<String>, CompleteError>) {
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
    ) -> Result<Option<String>, CompleteError> {
        let mut inner = self.inner.lock().expect("script");
        inner.calls.push(request.clone());
        inner.replies.pop_front().unwrap_or(Ok(None))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryClass {
    Timeout,
    Connect,
    Status(u16),
    Other,
}

pub fn should_retry(class: RetryClass) -> bool {
    match class {
        RetryClass::Timeout | RetryClass::Connect => true,
        RetryClass::Status(code) => matches!(code, 429 | 500..=599),
        RetryClass::Other => false,
    }
}

pub fn failure_message(class: RetryClass, total: Duration, detail: &str) -> CompleteError {
    let message = match class {
        RetryClass::Timeout => format!("model call timed out after {}s", total.as_secs()),
        RetryClass::Connect => "model call failed to connect".to_string(),
        RetryClass::Status(code) if detail.is_empty() => format!("model call returned {code}"),
        RetryClass::Status(code) => format!("model call returned {code}: {detail}"),
        RetryClass::Other => format!("model call failed: {detail}"),
    };
    CompleteError::Transport(message)
}

/// Longest provider error text kept on the error line, in characters.
pub const DETAIL_CHARS: usize = 200;

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
    if line.chars().count() <= DETAIL_CHARS {
        return line;
    }
    let mut cut: String = line.chars().take(DETAIL_CHARS).collect();
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

pub fn interpret_completion(text: &str) -> Result<Option<String>, CompleteError> {
    let value: Value =
        serde_json::from_str(text).map_err(|err| CompleteError::EmptyParse(err.to_string()))?;
    let choices = value
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| CompleteError::EmptyParse("choices".to_string()))?;
    let Some(choice) = choices.first() else {
        return Err(CompleteError::EmptyParse("choices".to_string()));
    };
    let Some(message) = choice.get("message") else {
        return Err(CompleteError::EmptyParse("message".to_string()));
    };
    let Some(calls) = message.get("tool_calls") else {
        return Ok(None);
    };
    if calls.is_null() {
        return Ok(None);
    }
    let calls = calls
        .as_array()
        .ok_or_else(|| CompleteError::EmptyParse("tool_calls".to_string()))?;
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
            return Err(CompleteError::EmptyParse("arguments".to_string()));
        };
        if let Some(text) = arguments.as_str() {
            if serde_json::from_str::<Value>(text).is_err() {
                return Err(CompleteError::EmptyParse("arguments".to_string()));
            }
            return Ok(Some(text.to_string()));
        }
        if arguments.is_object() {
            return Ok(Some(arguments.to_string()));
        }
        return Err(CompleteError::EmptyParse("arguments".to_string()));
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
        let parsed = interpret_completion(text).unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn act_arguments_are_returned() {
        let text = r#"{"choices":[{"message":{"role":"assistant","content":"ignored","tool_calls":[{"type":"function","function":{"name":"act","arguments":"{\"says\":[],\"intents\":[]}"}}]}}]}"#;
        let parsed = interpret_completion(text).unwrap().unwrap();
        assert!(parsed.contains("\"says\""));
    }

    #[test]
    fn retry_classifier_retries_timeout_connect_429_and_5xx_only() {
        assert!(should_retry(RetryClass::Timeout));
        assert!(should_retry(RetryClass::Connect));
        assert!(should_retry(RetryClass::Status(429)));
        assert!(should_retry(RetryClass::Status(500)));
        assert!(should_retry(RetryClass::Status(503)));
        assert!(should_retry(RetryClass::Status(599)));
        assert!(!should_retry(RetryClass::Status(400)));
        assert!(!should_retry(RetryClass::Status(401)));
        assert!(!should_retry(RetryClass::Status(404)));
        assert!(!should_retry(RetryClass::Status(418)));
        assert!(!should_retry(RetryClass::Other));
        assert_eq!(
            line(RetryClass::Timeout, 60),
            "model call timed out after 60s"
        );
        assert_eq!(
            line(RetryClass::Timeout, 1),
            "model call timed out after 1s"
        );
        assert_eq!(
            line(RetryClass::Connect, 60),
            "model call failed to connect"
        );
        assert_eq!(line(RetryClass::Status(401), 60), "model call returned 401");
        assert_eq!(line(RetryClass::Status(429), 60), "model call returned 429");
        assert_eq!(line(RetryClass::Status(500), 60), "model call returned 500");
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
        assert_eq!(detail.chars().count(), DETAIL_CHARS + 3, "{detail}");
        assert!(detail.ends_with("..."), "{detail}");
        assert_eq!(provider_detail("", key), "");
        assert_eq!(
            match failure_message(RetryClass::Status(401), Duration::from_secs(60), "bad key") {
                CompleteError::Transport(message) => message,
                CompleteError::EmptyParse(message) => message,
            },
            "model call returned 401: bad key"
        );
    }

    fn line(class: RetryClass, secs: u64) -> String {
        let detail = match class {
            RetryClass::Status(_) => "",
            _ => "detail",
        };
        match failure_message(class, Duration::from_secs(secs), detail) {
            CompleteError::Transport(message) => message,
            CompleteError::EmptyParse(message) => message,
        }
    }
}
