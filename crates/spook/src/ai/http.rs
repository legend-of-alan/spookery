use std::time::Duration;

use serde_json::{json, Value};

use crate::ai::fail;
use crate::ai::model::{CompleteError, CompletionRequest, Model};
use crate::ai::HostError;

pub struct Env {
    key: String,
    model: String,
    url: String,
}

pub struct HttpModel {
    env: Env,
    client: reqwest::blocking::Client,
}

pub fn require_env() -> Result<Env, HostError> {
    let key = match std::env::var("XAI_API_KEY") {
        Ok(value) => value,
        Err(_) => return Err(fail("XAI_API_KEY is unset")),
    };
    let model = match std::env::var("SPOOK_GM_MODEL") {
        Ok(value) => value,
        Err(_) => return Err(fail("SPOOK_GM_MODEL is unset")),
    };
    let url = std::env::var("SPOOK_GM_URL").unwrap_or_else(|_| "https://api.x.ai/v1".to_string());
    Ok(Env {
        key,
        model,
        url: url.trim_end_matches('/').to_string(),
    })
}

impl HttpModel {
    pub fn new(env: Env) -> Result<Self, HostError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3600))
            .build()
            .map_err(|err| fail(err.to_string()))?;
        Ok(Self { env, client })
    }
}

impl Model for HttpModel {
    fn complete(&mut self, request: &CompletionRequest) -> Result<Option<String>, CompleteError> {
        let body = chat_body(&self.env.model, &request.prompt, request.says_only);
        let url = format!("{}/chat/completions", self.env.url);
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.env.key)
            .json(&body)
            .send()
            .map_err(|err| CompleteError::Transport(err.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .map_err(|err| CompleteError::Transport(err.to_string()))?;
        if !status.is_success() {
            return Err(CompleteError::Transport(format!("{status} {text}")));
        }
        interpret_completion(&text)
    }
}

/// Chat-completions body. Fields follow the xAI chat-completions quickstart
/// (`messages`, `model`, `stream`) plus one function, `act`.
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
                    "required": ["actor_id", "actor_kind", "move", "target"],
                    "properties": {
                        "actor_id": { "type": "string" },
                        "actor_kind": { "type": "string", "enum": ["npc", "player"] },
                        "move": { "type": "string" },
                        "target": { "type": "object" }
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
    fn a_missing_key_does_not_dial() {
        let _guard = ENV.lock().unwrap_or_else(|err| err.into_inner());
        let saved_key = std::env::var("XAI_API_KEY").ok();
        let saved_model = std::env::var("SPOOK_GM_MODEL").ok();
        std::env::remove_var("XAI_API_KEY");
        std::env::set_var("SPOOK_GM_MODEL", "grok-test");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let err = crate::ai::sit(std::path::Path::new("."), &addr).expect_err("missing key");
        assert!(err.message.contains("XAI_API_KEY"), "{}", err.message);
        match listener.accept() {
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("listener saw a connection: {other:?}"),
        }
        std::env::set_var("XAI_API_KEY", "test-key");
        std::env::remove_var("SPOOK_GM_MODEL");
        let err = crate::ai::sit(std::path::Path::new("."), &addr).expect_err("missing model");
        assert!(err.message.contains("SPOOK_GM_MODEL"), "{}", err.message);
        match listener.accept() {
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("listener saw a connection: {other:?}"),
        }
        restore("XAI_API_KEY", saved_key);
        restore("SPOOK_GM_MODEL", saved_model);
    }

    fn restore(name: &str, value: Option<String>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
