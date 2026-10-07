use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionRequest {
    pub prompt: String,
    pub says_only: bool,
}

#[derive(Debug)]
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
    fn complete(&mut self, request: &CompletionRequest) -> Result<Option<String>, CompleteError>;
}

#[derive(Clone, Default)]
pub struct Script {
    inner: Arc<Mutex<ScriptInner>>,
}

#[derive(Default)]
struct ScriptInner {
    replies: VecDeque<Result<Option<String>, CompleteError>>,
    calls: Vec<CompletionRequest>,
}

impl Script {
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

impl Model for Script {
    fn complete(&mut self, request: &CompletionRequest) -> Result<Option<String>, CompleteError> {
        let mut inner = self.inner.lock().expect("script");
        inner.calls.push(request.clone());
        inner.replies.pop_front().unwrap_or(Ok(None))
    }
}
