//! The human terminals, the GM client, and the AI GM runner.

use std::fmt;
use std::io;

pub mod ai;
pub mod gm_connection;
use spookery_protocol as protocol;
pub mod terminal;

/// A GM client failure, returned from the agent door client to `spook gm` and the AI GM
/// loop. Display is the line either prints for it.
#[derive(Debug)]
pub enum Error {
    /// The agent door could not be reached.
    Io(io::Error),
    /// The table refused the GM seat. Display is the refusal's sentence.
    Refused(protocol::Refusal),
    /// A tool reply that is neither its reply type nor a refusal, with the JSON the door sent.
    Unreadable(serde_json::Value),
    /// A tool call the MCP client could not finish, as its message.
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
