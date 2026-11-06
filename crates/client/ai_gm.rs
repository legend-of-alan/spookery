//! Sit the GM seat for `spookery_agent::run` over the WebSocket connection.

use std::io::Write;
use std::net::SocketAddr;

use spookery_agent::{Model, Seat, SeatError};
use tokio::sync::Mutex;

use crate::protocol::{ClientMessage, ServerMessage};
use crate::ws::Connection;
use crate::Error;

/// Sit the GM seat on the game port at `addr` and play each turn through `model` until the
/// host closes the seat. `listening` and each warning go to `err`, one line each.
pub async fn run(
    addr: SocketAddr,
    model: impl Model,
    err: impl Write + Send + 'static,
) -> Result<(), Error> {
    let (connection, _hello) = Connection::open(addr, "gm").await?;
    let err = std::sync::Mutex::new(err);
    let report = move |line: &str| {
        let mut err = err.lock().unwrap_or_else(|err| err.into_inner());
        let _ = writeln!(err, "{line}").and_then(|()| err.flush());
    };
    spookery_agent::run(model, WsSeat(Mutex::new(connection)), report).await?;
    Ok(())
}

/// The GM seat's connection, one adapter call at a time.
struct WsSeat(Mutex<Connection>);

impl Seat for WsSeat {
    async fn send(&self, message: ClientMessage) -> Result<(), SeatError> {
        let mut connection = self.0.lock().await;
        connection.send_message(&message).await.map_err(Into::into)
    }

    async fn recv(&self) -> Option<Result<ServerMessage, SeatError>> {
        let mut connection = self.0.lock().await;
        Some(connection.recv().await?.map_err(Into::into))
    }
}

impl From<Error> for SeatError {
    fn from(err: Error) -> Self {
        SeatError(err.to_string())
    }
}

impl From<spookery_agent::Error> for Error {
    fn from(err: spookery_agent::Error) -> Self {
        Error::Message(err.to_string())
    }
}
