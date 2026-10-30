//! The model client and the GM loop that calls it.

mod gm_loop;
mod http_model;

use std::net::SocketAddr;

pub use gm_loop::{run_gm, TurnWatch};
pub use http_model::{HttpModel, Limits, Settings, Timer, WallClock};
pub use spookery_agent::*;

use crate::gm_connection::ToolResult;

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

pub(crate) fn report_error(message: impl Into<String>) -> Error {
    let message = message.into();
    eprintln!("{message}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    Error { message }
}

/// Parse a JSON-RPC tool-result line from the GM seat into a [`ToolResult`].
pub fn parse_tool_result(raw: &str) -> Result<ToolResult, Error> {
    let body =
        crate::protocol::parse_tool_result(raw).map_err(|_| report_error(raw.to_string()))?;
    let refused = body.get("code").is_some();
    Ok(ToolResult {
        raw: raw.to_string(),
        body,
        refused,
    })
}

/// Sit the GM seat on `addr` and call the model.
///
/// A missing `SPOOK_GM_KEY`, `SPOOK_GM_URL`, or `SPOOK_GM_MODEL` is reported before the dial.
pub async fn sit(addr: SocketAddr, door: Option<SocketAddr>) -> Result<(), Error> {
    let env = http_model::Settings::from_env()?;
    let mut model = http_model::HttpModel::new(env)?;
    run_gm(addr, door, &mut model, &TurnWatch::default()).await
}
