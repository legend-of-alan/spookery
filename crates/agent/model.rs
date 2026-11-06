//! One model call: the turn's messages, the GM's tools, and the request/response codecs.

use std::fmt;
use std::future::Future;
use std::sync::LazyLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::protocol::CLIENT_SCHEMA;

/// Sent from the agent to its `Model`: one message of a turn's conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatMessage {
    /// The turn's opening prompt.
    User { content: String },
    /// One model reply, its tool calls echoed back as they came.
    Assistant { tool_calls: Vec<ToolCall> },
    /// The answer to the tool call `tool_call_id`.
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

/// One model call: the turn's messages in, the reply's tool calls out, none when the model
/// is done. The HTTP client and the tests' scripted models implement this.
pub trait Model: Send + Sync + 'static {
    fn complete(
        &self,
        messages: &[ChatMessage],
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

/// The GM's tools: the seat words `perceive`, `say` and `queue`, their parameters read from
/// the published client schema, and `end_turn`, which the agent answers with `commit`.
static TOOLS: LazyLock<Value> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(CLIENT_SCHEMA).expect("client schema");
    let tool = |name: &str, description: &str, parameters: &Value| {
        json!({
            "type": "function",
            "function": { "name": name, "description": description, "parameters": parameters }
        })
    };
    let args = |name: &str| &schema["$defs"][name];
    json!([
        tool(
            "perceive",
            "See what one actor sees now: where they are, who is there, what they carry and know. `as` is the actor's id.",
            args("PerceiveArgs"),
        ),
        tool(
            "say",
            "Speak to the table, to everyone in one place, or to one actor. No game time passes.",
            args("SayArgs"),
        ),
        tool(
            "queue",
            "Queue a move from the menu for an actor. The table answers queued or refused.",
            args("IntentArgs"),
        ),
        tool(
            "end_turn",
            "Play the round: the engine resolves every queued move and answers with the rolls. You may say what happened afterwards. Answer with no tool calls when you are done.",
            &json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ])
});

/// Chat-completions body: the turn's `messages`, `model`, `stream` and the GM's tools.
pub fn chat_body(model: &str, messages: &[ChatMessage]) -> Value {
    let messages: Vec<Value> = messages.iter().map(wire_message).collect();
    json!({
        "messages": messages,
        "model": model,
        "stream": false,
        "tools": *TOOLS
    })
}

fn wire_message(message: &ChatMessage) -> Value {
    match message {
        ChatMessage::User { content } => json!({ "role": "user", "content": content }),
        ChatMessage::Assistant { tool_calls } => {
            let calls: Vec<Value> = tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": { "name": call.name, "arguments": call.arguments }
                    })
                })
                .collect();
            json!({ "role": "assistant", "content": null, "tool_calls": calls })
        }
        ChatMessage::Tool {
            tool_call_id,
            content,
        } => json!({ "role": "tool", "tool_call_id": tool_call_id, "content": content }),
    }
}

/// Read a chat-completions reply: its tool calls in order, none when it has none. A call
/// without an id gets `call_<index>`, and arguments that are not a string keep their JSON
/// text, so the agent answers them itself.
pub fn parse_completion(text: &str) -> Result<Vec<ToolCall>, CompletionError> {
    let invalid = |what: &str| CompletionError::InvalidReply(what.to_string());
    let value: Value =
        serde_json::from_str(text).map_err(|err| CompletionError::InvalidReply(err.to_string()))?;
    let message = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .ok_or_else(|| invalid("choices"))?
        .get("message")
        .ok_or_else(|| invalid("message"))?;
    let calls = match message.get("tool_calls") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(calls) => calls.as_array().ok_or_else(|| invalid("tool_calls"))?,
    };
    calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let function = call.get("function").ok_or_else(|| invalid("function"))?;
            let name = function.get("name").and_then(Value::as_str);
            let arguments = match function.get("arguments") {
                Some(Value::String(text)) => text.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            Ok(ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .map_or_else(|| format!("call_{index}"), str::to_string),
                name: name.ok_or_else(|| invalid("name"))?.to_string(),
                arguments,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_are_the_gm_words_and_end_turn_with_schema_parameters() {
        let body = chat_body("grok-test", &[]);
        let tools = body["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["perceive", "say", "queue", "end_turn"]);
        let schema: Value = serde_json::from_str(CLIENT_SCHEMA).unwrap();
        for (tool, args) in tools.iter().zip(["PerceiveArgs", "SayArgs", "IntentArgs"]) {
            assert_eq!(tool["function"]["parameters"], schema["$defs"][args]);
        }
        assert!(!body.to_string().contains("$ref"), "{body}");
        assert_eq!(body["stream"], false);
        assert_eq!(body["model"], "grok-test");
    }

    #[test]
    fn each_tool_call_is_echoed_and_answered_by_its_id() {
        let call = ToolCall {
            id: "call_7".into(),
            name: "say".into(),
            arguments: r#"{"address":"table","text":"hi"}"#.into(),
        };
        let messages = [
            ChatMessage::User {
                content: "the table is quiet".into(),
            },
            ChatMessage::Assistant {
                tool_calls: vec![call],
            },
            ChatMessage::Tool {
                tool_call_id: "call_7".into(),
                content: "said".into(),
            },
        ];
        let body = chat_body("grok-test", &messages);
        assert_eq!(
            body["messages"],
            json!([
                { "role": "user", "content": "the table is quiet" },
                { "role": "assistant", "content": null, "tool_calls": [{
                    "id": "call_7",
                    "type": "function",
                    "function": { "name": "say", "arguments": r#"{"address":"table","text":"hi"}"# }
                }] },
                { "role": "tool", "tool_call_id": "call_7", "content": "said" }
            ])
        );
    }

    #[test]
    fn prose_without_a_tool_call_means_done() {
        let text = r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#;
        assert_eq!(parse_completion(text), Ok(Vec::new()));
        assert!(parse_completion("not json").is_err());
    }

    #[test]
    fn tool_calls_keep_their_order_ids_and_raw_arguments() {
        let text = r#"{"choices":[{"message":{"role":"assistant","tool_calls":[
            {"id":"a","type":"function","function":{"name":"queue","arguments":"{\"actor_id\":\"mira\",\"move\":\"wait\"}"}},
            {"type":"function","function":{"name":"end_turn","arguments":{}}},
            {"id":"c","type":"function","function":{"name":"perceive","arguments":"not json"}}
        ]}}]}"#;
        let calls = parse_completion(text).unwrap();
        let call = |id: &str, name: &str, arguments: &str| ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        };
        assert_eq!(
            calls,
            [
                call("a", "queue", r#"{"actor_id":"mira","move":"wait"}"#),
                call("call_1", "end_turn", "{}"),
                call("c", "perceive", "not json"),
            ]
        );
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
