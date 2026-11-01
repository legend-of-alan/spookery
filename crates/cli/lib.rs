//! The human terminals, the GM client, and the AI GM runner.

use std::fmt;
use std::io;

pub mod ai_gm;
pub mod gm;
pub mod http_model;
pub mod mcp;
pub mod play;
use spookery_protocol as protocol;
mod text;

/// A client failure: what `play::run`, `gm::run` and `ai_gm::run` return to `spook`, and what
/// the MCP client and `HttpModel` return to them. Display is the line `spook` prints for it.
#[derive(Debug)]
pub enum Error {
    /// A socket could not be dialed, read or written.
    Io(io::Error),
    /// The table refused the GM seat. Display is the refusal's sentence.
    Refused(protocol::Refusal),
    /// A tool reply that is neither its reply type nor a refusal, or an event that is no
    /// `GmEvent`, with the JSON the MCP endpoint sent.
    Unreadable(serde_json::Value),
    /// Any other failure, as the line to print.
    Message(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(err) => err.fmt(f),
            Error::Refused(refusal) => refusal.fmt(f),
            Error::Unreadable(body) => body.fmt(f),
            Error::Message(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for Error {}

/// Stdin lines from a plain thread. A read on `tokio::io::stdin` can't be
/// cancelled, so the runtime would wait on exit for the next Enter.
fn stdin_lines() -> tokio::sync::mpsc::UnboundedReceiver<String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in io::stdin().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    rx
}
