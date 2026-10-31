//! The client that sits the GM seat over the agent door.

use std::collections::VecDeque;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, CustomNotification,
    Implementation, ProtocolVersion,
};
use rmcp::service::{ClientInitializeError, NotificationContext, RoleClient, RunningService};
use rmcp::transport::common::client_side_sse::NeverRetry;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, Transport};
use rmcp::{ClientHandler, ServiceExt};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::protocol::{Reason, Refusal};

/// A GM-seat failure. The display text is the seat's message or the I/O error.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Invalid(String),
    Refused { refusal: Refusal, raw: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Invalid(line) => f.write_str(line),
            Error::Refused { raw, .. } => f.write_str(raw),
        }
    }
}

impl std::error::Error for Error {}

pub use crate::protocol::GmEvent;

/// One `spook/event` the seat already sent. `raw` is the notification line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub raw: String,
    pub body: GmEvent,
}

/// A message from the GM seat, before a client's own turn rules.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Result { id: u64, raw: String },
    Event(Event),
    Invalid(String),
    Refused { refusal: Refusal, raw: String },
    Eof,
}

/// The structured body of one tool result.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub raw: String,
    pub body: Value,
    pub refused: bool,
}

struct Inbox {
    incoming: mpsc::UnboundedSender<Incoming>,
}

impl ClientHandler for Inbox {
    fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        if notification.method == "spook/event" {
            let _ = self.incoming.send(read_event(notification.params.as_ref()));
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

/// The loopback GM seat over streamable HTTP.
pub struct Connection {
    service: RunningService<RoleClient, Inbox>,
    /// Finishes when the host closes the agent door.
    watch: tokio::task::JoinHandle<()>,
    incoming: mpsc::UnboundedReceiver<Incoming>,
    buffered: VecDeque<Incoming>,
    next_id: u64,
    initialize_line: String,
}

impl Connection {
    /// Dial `http://127.0.0.1:<port + 1>/mcp` and finish the handshake.
    pub async fn connect_default_door(addr: std::net::SocketAddr) -> Result<Self, Error> {
        dial(default_door(addr)?).await
    }

    /// Dial `http://127.0.0.1:<port>/mcp`. `door` is the agent door, not the game port.
    pub async fn connect_door(door: std::net::SocketAddr) -> Result<Self, Error> {
        dial(door).await
    }
}

fn default_door(addr: std::net::SocketAddr) -> Result<std::net::SocketAddr, Error> {
    crate::protocol::default_door(addr).ok_or_else(|| Error::Invalid(addr.to_string()))
}

async fn dial(door: std::net::SocketAddr) -> Result<Connection, Error> {
    let url = format!("http://{door}/mcp");
    let (events, incoming) = mpsc::unbounded_channel();
    // A closed loopback seat ends the stream. The default client retries
    // that stream forever, so a dropped `spook run` never looks closed.
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());
    config.retry_config = Arc::new(NeverRetry::default());
    config.reinit_on_expired_session = false;
    let transport = SeatTransport {
        inner: StreamableHttpClientTransport::from_config(config),
        incoming: events.clone(),
        closed: false,
    };
    let running = match (Inbox {
        incoming: events.clone(),
    })
    .serve(transport)
    .await
    {
        Ok(running) => running,
        // The streamable client skips a JSON-RPC error on the initialize
        // event stream and reports the stream as closed. Read that refusal
        // from a fresh response body.
        Err(err @ ClientInitializeError::ConnectionClosed(_)) => {
            return Err(probe_seat_refusal(&url).await.unwrap_or_else(|| err.into()));
        }
        Err(err) => return Err(err.into()),
    };
    // rmcp keeps no read on the agent port once a tool call's stream
    // ends, so a killed `spook run` never looks closed. A plain connection
    // with a pending read sees that close the way the TCP client did.
    let watch = tokio::spawn(async move {
        if let Ok(mut socket) = tokio::net::TcpStream::connect(door).await {
            let mut buf = [0u8; 1];
            tokio::select! {
                _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf) => {}
                () = events.closed() => return,
            }
        }
        let _ = events.send(Incoming::Eof);
    });
    Ok(Connection {
        initialize_line: initialize_line_of(&running),
        service: running,
        watch,
        incoming,
        buffered: VecDeque::new(),
        next_id: 2,
    })
}

impl Connection {
    /// The initialize result line, without a trailing newline.
    pub fn initialize_line(&self) -> &str {
        &self.initialize_line
    }

    pub async fn call(&mut self, name: &str, arguments: Value) -> Result<ToolResult, Error> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut outcome = call_tool(&self.service, name, arguments).await?;
        while let Ok(item) = self.incoming.try_recv() {
            self.buffered.push_back(item);
        }
        outcome.raw = result_line(id, &outcome.body);
        Ok(outcome)
    }

    /// Events read while waiting for the latest tool result, in arrival order.
    pub fn take_events(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while matches!(self.buffered.front(), Some(Incoming::Event(_))) {
            if let Some(Incoming::Event(event)) = self.buffered.pop_front() {
                out.push(event);
            }
        }
        out
    }

    pub async fn next_incoming(&mut self) -> Incoming {
        if let Some(item) = self.buffered.pop_front() {
            return item;
        }
        self.incoming.recv().await.unwrap_or(Incoming::Eof)
    }

    /// Buffered lines, then anything the reader has already queued.
    pub fn drain_ready(&mut self) -> Vec<Incoming> {
        let mut out: Vec<Incoming> = self.buffered.drain(..).collect();
        while let Ok(item) = self.incoming.try_recv() {
            out.push(item);
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

/// The streamable HTTP transport, plus one end-of-seat notice.
///
/// `receive` returning nothing is the seat going away. The TCP client turned
/// that into `Incoming::Eof`. Callers waiting on the next event use the same
/// signal to leave.
struct SeatTransport<T> {
    inner: T,
    incoming: mpsc::UnboundedSender<Incoming>,
    closed: bool,
}

impl<T> Transport<RoleClient> for SeatTransport<T>
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
            let _ = self.incoming.send(Incoming::Eof);
        }
        message
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}

fn initialize_line_of(running: &RunningService<RoleClient, Inbox>) -> String {
    let version = running
        .peer_info()
        .map(|info| info.protocol_version.as_str().to_string())
        .unwrap_or_default();
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "protocolVersion": version }
    })
    .to_string()
}

async fn call_tool(
    running: &RunningService<RoleClient, Inbox>,
    name: &str,
    arguments: Value,
) -> Result<ToolResult, Error> {
    let params = match arguments {
        Value::Object(map) => CallToolRequestParams::new(name.to_string()).with_arguments(map),
        Value::Null => CallToolRequestParams::new(name.to_string()),
        other => return Err(Error::Invalid(other.to_string())),
    };
    match running.call_tool(params).await {
        Ok(result) => Ok(tool_result(result)),
        Err(err) => Err(Error::Invalid(err.to_string())),
    }
}

fn tool_result(result: CallToolResult) -> ToolResult {
    let body = result.structured_content.clone().unwrap_or(Value::Null);
    let refused = body.get("code").is_some();
    ToolResult {
        raw: String::new(),
        body,
        refused,
    }
}

fn result_line(id: u64, body: &Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "structuredContent": body }
    })
    .to_string()
}

async fn probe_seat_refusal(url: &str) -> Option<Error> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    // Same version the seated client negotiated, so the probe cannot drift.
    // This initialize can sit a seat that freed after the first attempt.
    // The DELETE below gives that seat back.
    let version = ProtocolVersion::LATEST.as_str();
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{version}","capabilities":{{}},"clientInfo":{{"name":"spook-gm","version":"0"}}}}}}"#
    );
    let response = client
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .header("mcp-protocol-version", version)
        .body(body)
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
    let refusal = bracket_refusal(&text)?;
    Some(Error::Refused { refusal, raw: text })
}

impl From<ClientInitializeError> for Error {
    fn from(err: ClientInitializeError) -> Self {
        if let Some(refused) = refusal_of(&err) {
            return refused;
        }
        Error::Io(std::io::Error::other(err.to_string()))
    }
}

fn refusal_of(err: &ClientInitializeError) -> Option<Error> {
    match err {
        ClientInitializeError::JsonRpcError(data) => Some(parse_refused(data.message.as_ref())),
        ClientInitializeError::LegacyFallbackFailed { discover, fallback } => {
            refusal_of(discover).or_else(|| refusal_of(fallback))
        }
        other => {
            let text = other.to_string();
            bracket_refusal(&text).map(|refusal| Error::Refused { refusal, raw: text })
        }
    }
}

fn parse_refused(raw: &str) -> Error {
    let refusal = bracket_refusal(raw).unwrap_or_else(|| Reason::TableFull.into());
    Error::Refused {
        refusal,
        raw: raw.to_string(),
    }
}

/// The refusal whose code is the last `[...]` in `text`.
fn bracket_refusal(text: &str) -> Option<Refusal> {
    let start = text.rfind('[')?;
    let end = text.rfind(']')?;
    if end <= start + 1 {
        return None;
    }
    let code = &text[start + 1..end];
    if code.is_empty() || code.contains([' ', '[']) {
        return None;
    }
    Some(Refusal {
        code: code.to_string().into(),
        detail: None,
    })
}

/// Read one `spook/event` notification. A body that is not a say, a queued
/// move, or a check is `Incoming::Bad`.
pub fn read_event(params: Option<&Value>) -> Incoming {
    let params = params.cloned().unwrap_or(Value::Null);
    let raw = serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "spook/event",
        "params": &params,
    }))
    .unwrap_or_else(|_| params.to_string());
    match serde_json::from_value(params) {
        Ok(body) => Incoming::Event(Event { raw, body }),
        Err(_) => Incoming::Invalid(raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_game_port_dials_the_next_port() {
        let game = std::net::SocketAddr::from(([127, 0, 0, 1], 9737));
        let door = std::net::SocketAddr::from(([127, 0, 0, 1], 9738));
        assert_eq!(default_door(game).ok(), Some(door));
    }

    #[test]
    fn a_body_that_matches_no_known_type_is_bad() {
        let Incoming::Invalid(line) = read_event(Some(&serde_json::json!({"kind": "nope"}))) else {
            panic!("malformed event was dropped");
        };
        assert!(line.contains("\"kind\":\"nope\""), "{line}");
    }
}
