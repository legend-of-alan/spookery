mod gm_loop;
mod http;
mod llm;
mod prompt;
mod wire;

use std::path::Path;

pub use gm_loop::{run_ai_gm, Watch};
pub use http::{Clock, Env, HttpModel, Limits, WallClock};
pub use llm::{CompleteError, CompletionRequest, Model, Script};
pub use prompt::{
    from_canon, keep, parse_actions, prepare, render_prompt, ActionKind, HandledSay, MenuRow,
    OutgoingSay, Prepared, PromptSide, QueueItem, RollLine, TurnView, Wake,
};
pub use wire::parse_tool_line;

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

/// Sit the GM seat on `addr` and call the model.
///
/// A missing `SPOOK_GM_KEY`, `SPOOK_GM_URL`, or `SPOOK_GM_MODEL` is reported before the dial.
pub fn sit(pack: &Path, addr: &str) -> Result<(), HostError> {
    let env = http::require_env()?;
    let mut model = http::HttpModel::new(env)?;
    run_ai_gm(pack, addr, &mut model, &Watch::default())
}
