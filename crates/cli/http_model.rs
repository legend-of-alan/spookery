//! The HTTP client that calls the model.

use std::time::Duration;

use spookery_agent::{
    chat_body, parse_completion, provider_detail, CompletionError, CompletionRequest, FailureKind,
    Model, ModelReply,
};

use crate::Error;

/// The longest one attempt at a model call may take, connect to last byte.
const MODEL_CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest the connect may take within one attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The pause before the one retry of an attempt that failed in a way worth retrying.
const RETRY_BACKOFF: Duration = Duration::from_secs(2);

pub struct Settings {
    key: String,
    model: String,
    url: String,
}

pub struct HttpModel {
    settings: Settings,
    client: reqwest::Client,
}

fn env_var(name: &str) -> Result<String, Error> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(_) => Err(Error::Message(format!("{name} is unset"))),
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

fn kind_of(err: &reqwest::Error) -> FailureKind {
    if err.is_timeout() {
        FailureKind::Timeout
    } else if err.is_connect() {
        FailureKind::Connect
    } else {
        FailureKind::Other
    }
}

struct Attempt {
    retry: bool,
    result: Result<Option<ModelReply>, CompletionError>,
}

fn fail_transport(err: &reqwest::Error) -> Attempt {
    let kind = kind_of(err);
    Attempt {
        retry: kind.is_retryable(),
        result: Err(CompletionError::transport(
            kind,
            MODEL_CALL_TIMEOUT,
            &err.to_string(),
        )),
    }
}

fn time_out() -> Attempt {
    Attempt {
        retry: true,
        result: Err(CompletionError::transport(
            FailureKind::Timeout,
            MODEL_CALL_TIMEOUT,
            "",
        )),
    }
}

impl HttpModel {
    pub fn new(settings: Settings) -> Result<Self, Error> {
        // No reqwest total timeout: `attempt` holds the limit, and dropping
        // the request closes the socket.
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|err| Error::Message(err.to_string()))?;
        Ok(Self { settings, client })
    }

    async fn post_completion(&self, request: &CompletionRequest) -> Attempt {
        let body = chat_body(&self.settings.model, &request.prompt, request.says_only);
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
            Err(err) => return fail_transport(&err),
        };
        let status = response.status();
        if !status.is_success() {
            let kind = FailureKind::Status(status.as_u16());
            let body = response.text().await.unwrap_or_default();
            let detail = provider_detail(&body, &self.settings.key);
            return Attempt {
                retry: kind.is_retryable(),
                result: Err(CompletionError::transport(
                    kind,
                    MODEL_CALL_TIMEOUT,
                    &detail,
                )),
            };
        }
        let text = match response.text().await {
            Ok(text) => text,
            Err(err) => return fail_transport(&err),
        };
        Attempt {
            retry: false,
            result: parse_completion(&text, request.says_only),
        }
    }

    async fn attempt(&self, request: &CompletionRequest) -> Attempt {
        // A timed-out attempt is dropped before this returns, which closes its socket.
        match tokio::time::timeout(MODEL_CALL_TIMEOUT, self.post_completion(request)).await {
            Ok(attempt) => attempt,
            Err(_) => time_out(),
        }
    }
}

impl Model for HttpModel {
    async fn complete(
        &mut self,
        request: &CompletionRequest,
    ) -> Result<Option<ModelReply>, CompletionError> {
        let first = self.attempt(request).await;
        if !first.retry {
            return first.result;
        }
        tokio::time::sleep(RETRY_BACKOFF).await;
        self.attempt(request).await.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn each_missing_setting_is_named() {
        let _guard = ENV_LOCK.lock().await;
        let saved = save_settings();
        for missing in ["SPOOK_GM_KEY", "SPOOK_GM_URL", "SPOOK_GM_MODEL"] {
            clear_settings();
            for name in ["SPOOK_GM_KEY", "SPOOK_GM_URL", "SPOOK_GM_MODEL"] {
                if name != missing {
                    let val = if name == "SPOOK_GM_URL" {
                        "http://127.0.0.1:9"
                    } else {
                        "set"
                    };
                    std::env::set_var(name, val);
                }
            }
            let err = Settings::from_env().err().expect(missing);
            assert!(
                err.to_string().contains(&format!("{missing} is unset")),
                "{err}"
            );
        }
        clear_settings();
        let err = Settings::from_env().err().expect("all missing should fail");
        assert!(err.to_string().contains("SPOOK_GM_KEY is unset"), "{err}");
        restore_settings(saved);
    }

    #[tokio::test]
    async fn a_trailing_slash_is_trimmed() {
        let _guard = ENV_LOCK.lock().await;
        let saved = save_settings();
        clear_settings();
        std::env::set_var("SPOOK_GM_KEY", "test-key");
        std::env::set_var("SPOOK_GM_URL", "http://127.0.0.1:9/");
        std::env::set_var("SPOOK_GM_MODEL", "test-model");
        let env = Settings::from_env().expect("settings present");
        assert_eq!(env.key, "test-key");
        assert_eq!(env.url, "http://127.0.0.1:9");
        assert_eq!(env.model, "test-model");
        restore_settings(saved);
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
            match value {
                Some(value) => std::env::set_var(&name, value),
                None => std::env::remove_var(&name),
            }
        }
    }

    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
}
