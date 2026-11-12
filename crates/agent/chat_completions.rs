//! The chat-completions codec: the request body a model call sends, and the tool calls its
//! reply brings.

use serde_json::{json, Value};

use crate::model::{CompletionError, Message, ToolCall};
use crate::tool::TOOLS;

/// The chat-completions body: the turn's `messages`, `model`, `stream` and the GM's tools.
pub fn render_chat_body(model: &str, messages: &[Message]) -> Value {
    let messages: Vec<Value> = messages.iter().map(render_message).collect();
    json!({
        "messages": messages,
        "model": model,
        "stream": false,
        "tools": *TOOLS
    })
}

fn render_message(message: &Message) -> Value {
    match message {
        Message::User { content } => json!({ "role": "user", "content": content }),
        Message::Assistant { tool_calls } => {
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
        Message::Tool {
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
    use crate::protocol::CLIENT_SCHEMA;

    #[test]
    fn the_tools_are_the_gm_commands_and_end_turn_with_schema_parameters() {
        let body = render_chat_body("grok-test", &[]);
        let tools = body["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["perceive", "say", "queue", "end_turn"]);
        let schema: Value = serde_json::from_str(CLIENT_SCHEMA).unwrap();
        for (tool, args) in tools.iter().zip(["PerceiveArgs", "SayArgs", "QueueArgs"]) {
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
            Message::User {
                content: "the table is quiet".into(),
            },
            Message::Assistant {
                tool_calls: vec![call],
            },
            Message::Tool {
                tool_call_id: "call_7".into(),
                content: "said".into(),
            },
        ];
        let body = render_chat_body("grok-test", &messages);
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
}
