//! The client side of the GM seat over the MCP endpoint.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, ClientJsonRpcMessage, ClientRequest,
    CustomNotification, Implementation, InitializeRequest, JsonRpcError, ProtocolVersion,
    RequestId,
};
use rmcp::service::{ClientInitializeError, NotificationContext, RoleClient, RunningService};
use rmcp::transport::common::client_side_sse::NeverRetry;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, Transport};
use rmcp::{ClientHandler, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::protocol::{Refusal, ServerMessage, ToolResult};
use crate::Error;

/// One item on the event channel from the MCP endpoint's handlers to the `GmClient`: an
/// event, the error for one that could not be read, or `None` for the end of the seat.
type EventItem = Option<Result<ServerMessage, Error>>;

/// Hands each `spook/event` notification to the `GmClient` as an `EventItem`.
struct EventSender {
    tx: mpsc::UnboundedSender<EventItem>,
}

impl ClientHandler for EventSender {
    fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        if notification.method == "spook/event" {
            let _ = self.tx.send(Some(parse_event(notification.params)));
        }
        std::future::ready(())
    }

    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("spook-gm", "0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST)
    }
}

/// The client side of the GM seat: dials the MCP endpoint, sends GM words as tool calls, hears
/// events.
pub struct GmClient {
    service: RunningService<RoleClient, EventSender>,
    /// Finishes when the host closes the MCP endpoint.
    watch: tokio::task::JoinHandle<()>,
    rx: mpsc::UnboundedReceiver<EventItem>,
    buffered: VecDeque<EventItem>,
    protocol_version: String,
}

impl GmClient {
    /// Dial `http://<mcp_addr>/mcp` and finish the handshake. `mcp_addr` is the MCP address,
    /// not the game port.
    pub async fn connect(mcp_addr: SocketAddr) -> Result<Self, Error> {
        let url = format!("http://{mcp_addr}/mcp");
        let (tx, rx) = mpsc::unbounded_channel();
        // A closed loopback seat ends the stream. The default client retries
        // that stream forever, so a dropped `spook run` never looks closed.
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());
        config.retry_config = Arc::new(NeverRetry::default());
        config.reinit_on_expired_session = false;
        let transport = ClientTransport {
            inner: StreamableHttpClientTransport::from_config(config),
            tx: tx.clone(),
            closed: false,
        };
        let sender = EventSender { tx: tx.clone() };
        let info = sender.get_info();
        let service = match sender.serve(transport).await {
            Ok(service) => service,
            // The streamable client skips a JSON-RPC error on the initialize
            // event stream and reports the stream as closed. Read that refusal
            // from a fresh response body.
            Err(err @ ClientInitializeError::ConnectionClosed(_)) => {
                return Err(match probe_seat_refusal(&url, info).await {
                    Some(refusal) => Error::Refused(refusal),
                    None => err.into(),
                });
            }
            Err(err) => return Err(err.into()),
        };
        // rmcp keeps no read on the MCP port once a tool call's stream
        // ends, so a killed `spook run` never looks closed. A plain connection
        // with a pending read sees that close the way the TCP client did.
        let watch = tokio::spawn(async move {
            if let Ok(mut socket) = tokio::net::TcpStream::connect(mcp_addr).await {
                let mut buf = [0u8; 1];
                tokio::select! {
                    _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf) => {}
                    () = tx.closed() => return,
                }
            }
            let _ = tx.send(None);
        });
        let protocol_version = service
            .peer_info()
            .map(|info| info.protocol_version.as_str().to_string())
            .unwrap_or_default();
        Ok(GmClient {
            service,
            watch,
            rx,
            buffered: VecDeque::new(),
            protocol_version,
        })
    }

    /// The protocol version the MCP endpoint answered the initialize with.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    /// Send the word `word` as a tool call. The result is every message it sent this seat,
    /// its reply last. A result that is not one is `Error::Unreadable`.
    pub async fn call(
        &mut self,
        word: &str,
        args: &impl Serialize,
    ) -> Result<Vec<ServerMessage>, Error> {
        let arguments = match serde_json::to_value(args) {
            Ok(Value::Object(arguments)) => arguments,
            other => return Err(Error::Message(format!("{word} arguments: {other:?}"))),
        };
        let params = CallToolRequestParams::new(word.to_string()).with_arguments(arguments);
        let result = self
            .service
            .call_tool(params)
            .await
            .map_err(|err| Error::Message(err.to_string()))?;
        while let Ok(item) = self.rx.try_recv() {
            self.buffered.push_back(item);
        }
        let body = result.structured_content.unwrap_or(Value::Null);
        ToolResult::deserialize(&body)
            .map(|result| result.messages)
            .map_err(|_| Error::Unreadable(body))
    }

    /// Events read while waiting for the latest tool result, in arrival order.
    pub fn drain_events(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        while matches!(self.buffered.front(), Some(Some(Ok(_)))) {
            if let Some(Some(Ok(event))) = self.buffered.pop_front() {
                out.push(event);
            }
        }
        out
    }

    /// The next event, or the error for one that could not be read. `None` when the seat
    /// ends.
    pub async fn recv(&mut self) -> EventItem {
        if let Some(item) = self.buffered.pop_front() {
            return item;
        }
        self.rx.recv().await.flatten()
    }

    /// Buffered events, then any the reader has already queued. An end of the seat among
    /// them is dropped.
    pub fn drain(&mut self) -> Vec<Result<ServerMessage, Error>> {
        let mut out: Vec<Result<ServerMessage, Error>> =
            self.buffered.drain(..).flatten().collect();
        while let Ok(item) = self.rx.try_recv() {
            out.extend(item);
        }
        out
    }

    /// Give the seat back. The host has freed it when this returns.
    pub async fn close(self) {
        // A closed seat has nothing to end. Cancel would send the session
        // DELETE to the dead port, and Windows takes about 2s to refuse it.
        if !self.watch.is_finished() {
            let _ = self.service.cancel().await;
        }
    }
}

/// The client side of the streamable HTTP transport, plus one end-of-seat notice.
///
/// `receive` returning nothing is the seat going away. The transport sends `None` on the
/// event channel then, so `GmClient::recv` returns `None` and callers waiting on the next
/// event leave.
struct ClientTransport<T> {
    inner: T,
    tx: mpsc::UnboundedSender<EventItem>,
    closed: bool,
}

impl<T> Transport<RoleClient> for ClientTransport<T>
where
    T: Transport<RoleClient>,
{
    type Error = T::Error;

    fn send(
        &mut self,
        item: rmcp::service::TxJsonRpcMessage<RoleClient>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<rmcp::service::RxJsonRpcMessage<RoleClient>> {
        let message = self.inner.receive().await;
        if message.is_none() && !self.closed {
            self.closed = true;
            let _ = self.tx.send(None);
        }
        message
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}

/// Send the session's own initialize, `info`, again on a plain request and read the
/// refusal from the `data` of the JSON-RPC error in its event stream.
async fn probe_seat_refusal(url: &str, info: ClientConfig) -> Option<Refusal> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    // This initialize can sit a seat that freed after the first attempt.
    // The DELETE below gives that seat back.
    let version = info.protocol_version.as_str().to_string();
    let initialize = ClientJsonRpcMessage::request(
        ClientRequest::InitializeRequest(InitializeRequest::new(info)),
        RequestId::Number(1),
    );
    let response = client
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .header("mcp-protocol-version", version)
        .body(serde_json::to_string(&initialize).ok()?)
        .send()
        .await
        .ok()?;
    let session = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let text = response.text().await.ok()?;
    if let Some(session) = session {
        let _ = client
            .delete(url)
            .header("mcp-session-id", session)
            .send()
            .await;
    }
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .find_map(|data| serde_json::from_str::<JsonRpcError>(data.trim()).ok())
        .and_then(|reply| refusal_from(&ClientInitializeError::JsonRpcError(reply.error)))
}

impl From<ClientInitializeError> for Error {
    fn from(err: ClientInitializeError) -> Self {
        match refusal_from(&err) {
            Some(refusal) => Error::Refused(refusal),
            None => Error::Io(std::io::Error::other(err.to_string())),
        }
    }
}

/// The refusal the table puts in the `data` of the MCP error that ends an initialize.
fn refusal_from(err: &ClientInitializeError) -> Option<Refusal> {
    match err {
        ClientInitializeError::JsonRpcError(error) => {
            Refusal::deserialize(error.data.as_ref()?).ok()
        }
        ClientInitializeError::LegacyFallbackFailed { discover, fallback } => {
            refusal_from(discover).or_else(|| refusal_from(fallback))
        }
        _ => None,
    }
}

/// Read one `spook/event` notification's params, one `ServerMessage`. Params that are not one
/// are `Error::Unreadable`, carrying the notification.
fn parse_event(params: Option<Value>) -> Result<ServerMessage, Error> {
    let params = params.unwrap_or(Value::Null);
    ServerMessage::deserialize(&params).map_err(|_| Error::Unreadable(event_message(&params)))
}

/// One `spook/event` notification as its JSON-RPC message.
fn event_message(params: &impl Serialize) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "spook/event",
        "params": params,
    })
}

/// One `spook/event` notification as its JSON-RPC line.
pub fn event_line(params: &impl Serialize) -> String {
    event_message(params).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_that_matches_no_known_type_is_unreadable() {
        let Err(Error::Unreadable(message)) = parse_event(Some(serde_json::json!({"body": 1})))
        else {
            panic!("malformed event was dropped");
        };
        let line = message.to_string();
        assert!(line.contains("\"body\":1"), "{line}");
    }
}
