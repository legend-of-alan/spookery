mod http;
mod model;
mod prompt;
mod turn;
mod wire;

use std::path::Path;

pub use model::{CompleteError, CompletionRequest, Model, Script};
pub use prompt::{
    from_canon, keep, parse_actions, prepare, render, HandledSay, MenuRow, OutgoingSay, Prepared,
    PromptSide, QueueItem, Roll, Rule, TurnView, Wake,
};
pub use turn::{drive, Watch};
pub use wire::{parse_tool_line, CallOutcome, Seat};

#[derive(Debug)]
pub struct HostError {
    pub message: String,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HostError {}

pub(crate) fn fail(message: impl Into<String>) -> HostError {
    let message = message.into();
    eprintln!("{message}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    HostError { message }
}

/// Sit the GM seat on `127.0.0.1:9737` and call the model.
///
/// A missing `XAI_API_KEY` or `SPOOK_GM_MODEL` is reported before the dial.
pub fn sit(pack: &Path) -> Result<(), HostError> {
    let env = http::require_env()?;
    let mut model = http::HttpModel::new(env)?;
    drive(pack, "127.0.0.1:9737", &mut model, &Watch::default())
}
