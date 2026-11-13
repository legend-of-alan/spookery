//! The GM's connection to the server, as the client implements it.

use std::fmt;
use std::future::Future;

use crate::protocol::{ClientMessage, ServerMessage};

/// The GM's connection to the server: the server's one ordered stream of messages, as the
/// client carries it.
pub trait Connection: Send + Sync + 'static {
    fn send(
        &self,
        message: ClientMessage,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send;
    /// The server's next message. `None` when the server closed the connection.
    fn recv(&self) -> impl Future<Output = Option<Result<ServerMessage, ConnectionError>>> + Send;
}

/// Returned from a client's `Connection` to the agent: why it could not send or read.
/// Display is the client's line for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionError(pub String);

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
