use std::future::Future;
use std::time::Duration;

use serde_json::{json, Value};

use crate::ai::fail;
use crate::ai::llm::{CompleteError, CompletionRequest, Model};
use crate::ai::HostError;

pub struct Env {
    key: String,
    model: String,
    url: String,
}

/// Fixed bounds for one model call. Production uses 60s total, 10s to connect,
/// and one retry after 2s. Tests pass a shorter total, and a clock, through
/// `with_limits`.
pub struct Limits {
    pub total: Duration,
    pub connect: Duration,
    pub backoff: Duration,
}

/// Time source for one model call's limit and the pause before its retry.
///
/// [`WallClock`] sleeps. A test clock's `wait` finishes when the test says the
/// time has passed, so a hung call can time out without a real delay.
pub trait Clock: Send + Sync {
    fn wait(&self, duration: Duration) -> impl Future<Output = ()> + Send;

    /// The attempt's socket has been dropped.
    ///
    /// The wall clock returns at once. A test clock can wait until its stub
    /// has observed the close, which keeps a later commit after that close.
    fn abandon(&self) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}

/// Production clock. `wait` sleeps for the duration.
#[derive(Clone, Copy, Debug, Default)]
pub struct WallClock;

impl Clock for WallClock {
    fn wait(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(duration)
    }
}

impl Limits {
    pub const fn production() -> Self {
        Self {
            total: Duration::from_secs(60),
            connect: Duration::from_secs(10),
            backoff: Duration::from_secs(2),
        }
    }
}

pub struct HttpModel<C> {
    env: Env,
    client: reqwest::Client,
    limits: Limits,
    clock: C,
    /// Dropped after `client`. The client may still be settling a connection.
    runtime: tokio::runtime::Runtime,
}

pub fn require_env() -> Result<Env, HostError> {
    let key = env_var("SPOOK_GM_KEY")?;
    let url = env_var("SPOOK_GM_URL")?;
    let model = env_var("SPOOK_GM_MODEL")?;
    Ok(Env::new(key, url, model))
}

fn env_var(name: &str) -> Result<String, HostError> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(_) => Err(fail(format!("{name} is unset"))),
    }
}

impl Env {
    pub fn new(key: impl Into<String>, url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            model: model.into(),
            url: trim_url(&url.into()),
        }
    }
}

fn trim_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

impl HttpModel<WallClock> {
    pub fn new(env: Env) -> Result<Self, HostError> {
        Self::with_limits(env, Limits::production(), WallClock)
    }
}

impl<C: Clock> HttpModel<C> {
    pub fn with_limits(env: Env, limits: Limits, clock: C) -> Result<Self, HostError> {
        // No reqwest total timeout. That timer is the wall clock. `clock.wait`
        // is the limit, and dropping the request closes the socket.
        let client = reqwest::Client::builder()
            .connect_timeout(limits.connect)
            .build()
            .map_err(|err| fail(err.to_string()))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|err| fail(err.to_string()))?;
        Ok(Self {
            env,
            client,
            limits,
            clock,
            runtime,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetryClass {
    Timeout,
    Connect,
    Status(u16),
    Other,
}

fn should_retry(class: RetryClass) -> bool {
    match class {
        RetryClass::Timeout | RetryClass::Connect => true,
        RetryClass::Status(code) => matches!(code, 429 | 500..=599),
        RetryClass::Other => false,
    }
}

fn class_of(err: &reqwest::Error) -> RetryClass {
    if err.is_timeout() {
        RetryClass::Timeout
    } else if err.is_connect() {
        RetryClass::Connect
    } else {
        RetryClass::Other
    }
}

fn failure_message(class: RetryClass, total: Duration, detail: &str) -> CompleteError {
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
const DETAIL_CHARS: usize = 200;

/// The provider's error body as one short line. The key is replaced before the
/// cut so no part of it can survive, and control characters and runs of
/// whitespace collapse to single spaces.
fn provider_detail(body: &str, key: &str) -> String {
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

struct Tried {
    retry: bool,
    result: Result<Option<String>, CompleteError>,
}

impl<C: Clock> HttpModel<C> {
    fn fail_transport(&self, err: &reqwest::Error) -> Tried {
        let class = class_of(err);
        Tried {
            retry: should_retry(class),
            result: Err(failure_message(class, self.limits.total, &err.to_string())),
        }
    }

    fn miss_deadline(&self) -> Tried {
        Tried {
            retry: true,
            result: Err(failure_message(RetryClass::Timeout, self.limits.total, "")),
        }
    }

    async fn read_attempt(&self, request: &CompletionRequest) -> Tried {
        let body = chat_body(&self.env.model, &request.prompt, request.says_only);
        let url = format!("{}/chat/completions", self.env.url);
        let response = match self
            .client
            .post(url)
            .bearer_auth(&self.env.key)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => return self.fail_transport(&err),
        };
        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            let class = RetryClass::Status(code);
            let body = response.text().await.unwrap_or_default();
            let detail = provider_detail(&body, &self.env.key);
            return Tried {
                retry: should_retry(class),
                result: Err(failure_message(class, self.limits.total, &detail)),
            };
        }
        let text = match response.text().await {
            Ok(text) => text,
            Err(err) => return self.fail_transport(&err),
        };
        Tried {
            retry: false,
            result: interpret_completion(&text),
        }
    }

    async fn attempt(&self, request: &CompletionRequest) -> Tried {
        // The block owns the request. Leaving it drops the socket before
        // `abandon`, so a test clock can see the close before this returns.
        {
            let attempt = self.read_attempt(request);
            tokio::pin!(attempt);
            let limit = self.clock.wait(self.limits.total);
            tokio::pin!(limit);
            tokio::select! {
                biased;
                result = &mut attempt => return result,
                _ = &mut limit => {}
            }
        }
        self.clock.abandon().await;
        self.miss_deadline()
    }
}

impl<C: Clock> Model for HttpModel<C> {
    fn complete(&mut self, request: &CompletionRequest) -> Result<Option<String>, CompleteError> {
        self.runtime.block_on(async {
            let first = self.attempt(request).await;
            if !first.retry {
                return first.result;
            }
            self.clock.wait(self.limits.backoff).await;
            self.attempt(request).await.result
        })
    }
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
    fn each_missing_setting_is_named_and_does_not_dial() {
        let _guard = ENV.lock().unwrap_or_else(|err| err.into_inner());
        let saved = save_settings();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        for missing in ["SPOOK_GM_KEY", "SPOOK_GM_URL", "SPOOK_GM_MODEL"] {
            clear_settings();
            for name in ["SPOOK_GM_KEY", "SPOOK_GM_URL", "SPOOK_GM_MODEL"] {
                if name != missing {
                    std::env::set_var(
                        name,
                        if name == "SPOOK_GM_URL" {
                            "http://127.0.0.1:9"
                        } else {
                            "set"
                        },
                    );
                }
            }
            let err = crate::ai::sit(std::path::Path::new("."), &addr, None).expect_err(missing);
            assert!(
                err.message.contains(&format!("{missing} is unset")),
                "{}",
                err.message
            );
            match listener.accept() {
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                other => panic!("listener saw a connection: {other:?}"),
            }
        }
        clear_settings();
        let err = match require_env() {
            Ok(_) => panic!("all missing should fail"),
            Err(err) => err,
        };
        assert!(
            err.message.contains("SPOOK_GM_KEY is unset"),
            "{}",
            err.message
        );
        restore_settings(saved);
    }

    #[test]
    fn a_trailing_slash_is_trimmed() {
        let _guard = ENV.lock().unwrap_or_else(|err| err.into_inner());
        let saved = save_settings();
        clear_settings();
        std::env::set_var("SPOOK_GM_KEY", "test-key");
        std::env::set_var("SPOOK_GM_URL", "http://127.0.0.1:9/");
        std::env::set_var("SPOOK_GM_MODEL", "test-model");
        let env = require_env().expect("settings present");
        assert_eq!(env.key, "test-key");
        assert_eq!(env.url, "http://127.0.0.1:9");
        assert_eq!(env.model, "test-model");
        restore_settings(saved);
    }

    #[test]
    fn production_limits_are_sixty_ten_and_two() {
        let model = HttpModel::new(Env::new("k", "http://127.0.0.1:9", "m")).expect("client");
        assert_eq!(model.limits.total, Duration::from_secs(60));
        assert_eq!(model.limits.connect, Duration::from_secs(10));
        assert_eq!(model.limits.backoff, Duration::from_secs(2));
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

    fn clear_settings() {
        for name in ["SPOOK_GM_KEY", "SPOOK_GM_URL", "SPOOK_GM_MODEL"] {
            std::env::remove_var(name);
        }
    }

    fn save_settings() -> Vec<(String, Option<String>)> {
        ["SPOOK_GM_KEY", "SPOOK_GM_URL", "SPOOK_GM_MODEL"]
            .into_iter()
            .map(|name| (name.to_string(), std::env::var(name).ok()))
            .collect()
    }

    fn restore_settings(saved: Vec<(String, Option<String>)>) {
        for (name, value) in saved {
            restore(&name, value);
        }
    }

    fn restore(name: &str, value: Option<String>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
