//! The model client and the GM loop that calls it.

mod gm_loop;
mod llm_http;

use std::net::SocketAddr;
use std::path::Path;

pub use gm_loop::{run_gm, TurnWatch};
pub use llm_http::{Clock, Env, HttpModel, Limits, WallClock};
pub use spookery_agent::*;

use crate::gm_connection::ToolCallResult;

#[derive(Debug)]
pub struct Error {
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub(crate) fn fail(message: impl Into<String>) -> Error {
    let message = message.into();
    eprintln!("{message}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    Error { message }
}

/// Parse a JSON-RPC tool-result line from the GM seat into a [`ToolCallResult`].
pub fn parse_tool_line(raw: &str) -> Result<ToolCallResult, Error> {
    let body = crate::protocol::read_tool_result(raw).map_err(|_| fail(raw.to_string()))?;
    let refused = body.get("code").is_some();
    Ok(ToolCallResult {
        raw: raw.to_string(),
        body,
        refused,
    })
}

/// Sit the GM seat on `addr` and call the model.
///
/// A missing `SPOOK_GM_KEY`, `SPOOK_GM_URL`, or `SPOOK_GM_MODEL` is reported before the dial.
pub fn sit(pack: &Path, addr: SocketAddr, door: Option<SocketAddr>) -> Result<(), Error> {
    let env = llm_http::require_env()?;
    let mut model = llm_http::HttpModel::new(env)?;
    run_gm(pack, addr, door, &mut model, &TurnWatch::default())
}
