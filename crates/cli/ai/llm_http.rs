//! The HTTP client that calls the model.

use std::future::Future;
use std::time::Duration;

use spookery_agent::llm::{
    chat_body, failure_message, interpret_completion, provider_detail, should_retry, CompleteError,
    CompletionRequest, Model, RetryClass,
};

use crate::ai::{fail, Error};

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
}

pub fn require_env() -> Result<Env, Error> {
    let key = env_var("SPOOK_GM_KEY")?;
    let url = env_var("SPOOK_GM_URL")?;
    let model = env_var("SPOOK_GM_MODEL")?;
    Ok(Env::new(key, url, model))
}

fn env_var(name: &str) -> Result<String, Error> {
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
            url: url.into().trim_end_matches('/').to_string(),
        }
    }
}

impl HttpModel<WallClock> {
    pub fn new(env: Env) -> Result<Self, Error> {
        Self::with_limits(env, Limits::production(), WallClock)
    }
}

impl<C: Clock> HttpModel<C> {
    pub fn with_limits(env: Env, limits: Limits, clock: C) -> Result<Self, Error> {
        // No reqwest total timeout. That timer is the wall clock. `clock.wait`
        // is the limit, and dropping the request closes the socket.
        let client = reqwest::Client::builder()
            .connect_timeout(limits.connect)
            .build()
            .map_err(|err| fail(err.to_string()))?;
        Ok(Self {
            env,
            client,
            limits,
            clock,
        })
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

struct AttemptOutcome {
    retry: bool,
    result: Result<Option<String>, CompleteError>,
}

impl<C: Clock> HttpModel<C> {
    fn fail_transport(&self, err: &reqwest::Error) -> AttemptOutcome {
        let class = class_of(err);
        AttemptOutcome {
            retry: should_retry(class),
            result: Err(failure_message(class, self.limits.total, &err.to_string())),
        }
    }

    fn miss_deadline(&self) -> AttemptOutcome {
        AttemptOutcome {
            retry: true,
            result: Err(failure_message(RetryClass::Timeout, self.limits.total, "")),
        }
    }

    async fn read_attempt(&self, request: &CompletionRequest) -> AttemptOutcome {
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
            let class = RetryClass::Status(status.as_u16());
            let body = response.text().await.unwrap_or_default();
            let detail = provider_detail(&body, &self.env.key);
            return AttemptOutcome {
                retry: should_retry(class),
                result: Err(failure_message(class, self.limits.total, &detail)),
            };
        }
        let text = match response.text().await {
            Ok(text) => text,
            Err(err) => return self.fail_transport(&err),
        };
        AttemptOutcome {
            retry: false,
            result: interpret_completion(&text),
        }
    }

    async fn attempt(&self, request: &CompletionRequest) -> AttemptOutcome {
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
    async fn complete(
        &mut self,
        request: &CompletionRequest,
    ) -> Result<Option<String>, CompleteError> {
        let first = self.attempt(request).await;
        if !first.retry {
            return first.result;
        }
        self.clock.wait(self.limits.backoff).await;
        self.attempt(request).await.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn each_missing_setting_is_named_and_does_not_dial() {
        let _guard = ENV.lock().await;
        let saved = save_settings();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
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
            let err = crate::ai::sit(addr, None).await.expect_err(missing);
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
        let err = require_env().err().expect("all missing should fail");
        assert!(
            err.message.contains("SPOOK_GM_KEY is unset"),
            "{}",
            err.message
        );
        restore_settings(saved);
    }

    #[tokio::test]
    async fn a_trailing_slash_is_trimmed() {
        let _guard = ENV.lock().await;
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

    static ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
}
