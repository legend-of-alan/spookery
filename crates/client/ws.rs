//! The client side of a seat over `/player` or `/gm` on the game port.

use std::io::ErrorKind;
use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::protocol::{ClientMessage, ServerBody, ServerMessage};
use crate::Error;

/// One open seat connection: `/player` or `/gm` on the game port.
pub struct Connection {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl Connection {
    /// Dial `ws://<addr>/<seat>` and read the hello. A refused GM seat is
    /// `Error::Refused`.
    pub async fn open(addr: SocketAddr, seat: &str) -> Result<(Self, ServerMessage), Error> {
        let url = format!("ws://{addr}/{seat}");
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|err| Error::Io(std::io::Error::other(err)))?;
        let mut connection = Self { socket, next_id: 1 };
        match connection.recv().await {
            Some(Ok(ServerMessage {
                body: ServerBody::Refused(refusal),
                ..
            })) => Err(Error::Refused(refusal)),
            Some(Ok(hello)) => Ok((connection, hello)),
            Some(Err(err)) => Err(err),
            None => Err(Error::Message("NOT_IN_SPEC".into())),
        }
    }

    /// Send `command` with `args`, a JSON object, under the next id. Returns that id.
    pub async fn send(&mut self, command: &str, args: Value) -> Result<String, Error> {
        let id = format!("c{}", self.next_id);
        self.next_id += 1;
        self.send_message(&ClientMessage::new(&id, command, args))
            .await?;
        Ok(id)
    }

    /// Send one message as it is.
    pub async fn send_message(&mut self, message: &ClientMessage) -> Result<(), Error> {
        let text = serde_json::to_string(message).map_err(|err| Error::Message(err.to_string()))?;
        self.socket
            .send(Message::text(text))
            .await
            .map_err(|err| Error::Io(std::io::Error::other(err)))
    }

    /// Send `release` and wait for its reply, so the seat is free once this returns. A table
    /// that is already gone holds nothing to give back.
    pub async fn release(&mut self) -> Result<(), Error> {
        let Ok(id) = self.send("release", serde_json::json!({})).await else {
            return Ok(());
        };
        while let Some(Ok(message)) = self.recv().await {
            if message.reply_to.as_deref() == Some(id.as_str()) {
                break;
            }
        }
        Ok(())
    }

    /// The next message, or the text of one that could not be read as `Error::Unreadable`.
    /// `None` when the host closes the connection, with or without a close frame or by a reset, and
    /// `Error::Io` when the connection fails. Cancel safe.
    pub async fn recv(&mut self) -> Option<Result<ServerMessage, Error>> {
        loop {
            let text = match self.socket.next().await? {
                Ok(Message::Text(text)) => text,
                Ok(Message::Close(_))
                | Err(
                    WsError::ConnectionClosed
                    | WsError::AlreadyClosed
                    | WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake),
                ) => return None,
                // Windows reports a host that died as a reset, where Linux reads an end.
                Err(WsError::Io(err))
                    if matches!(
                        err.kind(),
                        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
                    ) =>
                {
                    return None
                }
                Err(err) => return Some(Err(Error::Io(std::io::Error::other(err)))),
                Ok(_) => continue,
            };
            return Some(serde_json::from_str(&text).map_err(|_| {
                Error::Unreadable(
                    serde_json::from_str(&text).unwrap_or(Value::String(text.to_string())),
                )
            }));
        }
    }
}
