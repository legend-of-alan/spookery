//! The HTTP client that calls the model.

use std::time::{Duration, Instant};

use spookery_agent::{chat_completions, model, CompletionError, Model, ToolCall};

use crate::Error;

/// The longest one attempt at a model call may take, connect to last byte.
pub const MODEL_CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest the connect may take within one attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The pause before the one retry of an attempt that failed in a way worth retrying.
pub const RETRY_BACKOFF: Duration = Duration::from_secs(2);
/// Longest provider error text kept on the error line, in characters.
const MAX_DETAIL_CHARS: usize = 200;

pub struct Settings {
    key: String,
    model: String,
    url: String,
}

pub struct HttpModel {
    settings: Settings,
    client: reqwest::Client,
}

fn env_var(var: &str) -> Result<String, Error> {
    match std::env::var(var) {
        Ok(value) => Ok(value),
        Err(_) => Err(Error::Message(format!("{var} is unset"))),
    }
}

impl Settings {
    pub fn from_env() -> Result<Self, Error> {
        let key = env_var("SPOOK_GM_KEY")?;
        let url = env_var("SPOOK_GM_URL")?;
        let model = env_var("SPOOK_GM_MODEL")?;
        Ok(Settings::new(key, url, model))
    }

    pub fn new(key: impl Into<String>, url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            model: model.into(),
            url: url.into().trim_end_matches('/').to_string(),
        }
    }
}

/// How a model call failed on the transport.
#[derive(Clone, Copy, Debug)]
enum TransportFailure {
    Timeout,
    Connect,
    Status(u16),
    Other,
}

impl TransportFailure {
    /// A timeout, a failed connect, 429 and 5xx are worth one more attempt.
    fn is_retryable(self) -> bool {
        match self {
            TransportFailure::Timeout | TransportFailure::Connect => true,
            TransportFailure::Status(code) => matches!(code, 429 | 500..=599),
            TransportFailure::Other => false,
        }
    }

    /// The error line for a model call that failed this way, with the provider's `detail`.
    fn to_error(self, detail: &str) -> CompletionError {
        let message = match self {
            TransportFailure::Timeout => {
                format!(
                    "model call timed out after {}s",
                    MODEL_CALL_TIMEOUT.as_secs()
                )
            }
            TransportFailure::Connect => "model call failed to connect".to_string(),
            TransportFailure::Status(code) if detail.is_empty() => {
                format!("model call returned {code}")
            }
            TransportFailure::Status(code) => format!("model call returned {code}: {detail}"),
            TransportFailure::Other => format!("model call failed: {detail}"),
        };
        CompletionError::Transport(message)
    }
}

impl From<&reqwest::Error> for TransportFailure {
    fn from(err: &reqwest::Error) -> Self {
        if err.is_timeout() {
            TransportFailure::Timeout
        } else if err.is_connect() {
            TransportFailure::Connect
        } else {
            TransportFailure::Other
        }
    }
}

/// The provider's error body as one short line. The key is replaced before the
/// cut so no part of it can survive, and control characters and runs of
/// whitespace collapse to single spaces.
fn redact_error_body(body: &str, key: &str) -> String {
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

struct Attempt {
    retry: bool,
    result: Result<chat_completions::Reply, CompletionError>,
}

impl Attempt {
    fn from_failure(failure: TransportFailure, detail: &str) -> Self {
        Attempt {
            retry: failure.is_retryable(),
            result: Err(failure.to_error(detail)),
        }
    }
}

impl HttpModel {
    pub fn new(settings: Settings) -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|err| Error::Message(err.to_string()))?;
        Ok(Self { settings, client })
    }

    async fn post_chat_completion(&self, messages: &[model::Message]) -> Attempt {
        let body = chat_completions::render_request(&self.settings.model, messages);
        let url = format!("{}/chat/completions", self.settings.url);
        let response = match self
            .client
            .post(url)
            .bearer_auth(&self.settings.key)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                return Attempt::from_failure(TransportFailure::from(&err), &err.to_string())
            }
        };
        let status = response.status();
        if !status.is_success() {
            let failure = TransportFailure::Status(status.as_u16());
            let body = response.text().await.unwrap_or_default();
            let detail = redact_error_body(&body, &self.settings.key);
            return Attempt::from_failure(failure, &detail);
        }
        let text = match response.text().await {
            Ok(text) => text,
            Err(err) => {
                return Attempt::from_failure(TransportFailure::from(&err), &err.to_string())
            }
        };
        Attempt {
            retry: false,
            result: chat_completions::parse_reply(&text),
        }
    }

    async fn attempt(&self, messages: &[model::Message]) -> Attempt {
        match tokio::time::timeout(MODEL_CALL_TIMEOUT, self.post_chat_completion(messages)).await {
            Ok(attempt) => attempt,
            Err(_) => Attempt::from_failure(TransportFailure::Timeout, ""),
        }
    }
}

impl Model for HttpModel {
    async fn request_tool_calls(
        &self,
        messages: &[model::Message],
    ) -> Result<Vec<ToolCall>, CompletionError> {
        let started = Instant::now();
        let mut attempts = 1;
        let mut attempt = self.attempt(messages).await;
        if let (true, Err(error)) = (attempt.retry, &attempt.result) {
            let elapsed_ms = started.elapsed().as_millis();
            tracing::warn!(%error, elapsed_ms, "model call failed; retrying once");
            tokio::time::sleep(RETRY_BACKOFF).await;
            attempts = 2;
            attempt = self.attempt(messages).await;
        }
        let elapsed_ms = started.elapsed().as_millis();
        let model_id = self.settings.model.as_str();
        match &attempt.result {
            Ok(chat_completions::Reply { tool_calls, usage }) => tracing::info!(
                model_id,
                attempts,
                elapsed_ms,
                tool_call_count = tool_calls.len(),
                prompt_tokens = usage.prompt_tokens,
                completion_tokens = usage.completion_tokens,
                cached_tokens = usage.cached_tokens,
                reasoning_tokens = usage.reasoning_tokens,
                "model call answered"
            ),
            Err(error) => {
                tracing::info!(model_id, attempts, elapsed_ms, %error, "model call failed");
            }
        }
        attempt.result.map(|reply| reply.tool_calls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scenarios cover a timeout and a 500 that retry, and a 401 that does not and
    /// whose line is redacted. These are the failures they never meet.
    #[test]
    fn connect_429_and_5xx_retry_and_each_failure_has_its_line() {
        let retried = [
            TransportFailure::Connect,
            TransportFailure::Status(429),
            TransportFailure::Status(599),
        ];
        for failure in retried {
            assert!(failure.is_retryable(), "{failure:?}");
        }
        for failure in [TransportFailure::Status(404), TransportFailure::Other] {
            assert!(!failure.is_retryable(), "{failure:?}");
        }
        let line = |failure: TransportFailure, detail| failure.to_error(detail).to_string();
        assert_eq!(
            line(TransportFailure::Connect, ""),
            "model call failed to connect"
        );
        assert_eq!(
            line(TransportFailure::Status(429), ""),
            "model call returned 429"
        );
        assert_eq!(
            line(TransportFailure::Status(404), "gone"),
            "model call returned 404: gone"
        );
        assert_eq!(
            line(TransportFailure::Other, "body"),
            "model call failed: body"
        );
    }

    #[test]
    fn a_long_error_body_is_cut_after_the_key_is_replaced() {
        let key = "xai-SECRETKEY123";
        let long = format!("{}{key}{}", "a".repeat(195), "b".repeat(300));
        let detail = redact_error_body(&long, key);
        assert!(!detail.contains("xai-"), "{detail}");
        assert_eq!(detail.chars().count(), MAX_DETAIL_CHARS + 3, "{detail}");
        assert!(detail.ends_with("..."), "{detail}");
    }
}
