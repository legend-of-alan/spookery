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

/// A GM-seat failure. The display text is the seat's message or the I/O error.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Bad(String),
    Refused { code: String, raw: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Bad(line) => f.write_str(line),
            Error::Refused { raw, .. } => f.write_str(raw),
        }
    }
}

impl std::error::Error for Error {}

pub use crate::protocol::EventBody;

/// One `spook/event` the seat already sent. `raw` is the notification line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub raw: String,
    pub body: EventBody,
}

/// A message from the GM seat, before a client's own turn rules.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Result { id: u64, raw: String },
    Event(Event),
    Bad(String),
    Refused { code: String, raw: String },
    Eof,
}

/// The structured body of one tool result.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallResult {
    pub raw: String,
    pub body: Value,
    pub refused: bool,
}

struct Inbox {
    events: mpsc::UnboundedSender<Incoming>,
}

impl ClientHandler for Inbox {
    fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        if notification.method == "spook/event" {
            let _ = self.events.send(read_event(notification.params.as_ref()));
        }
        std::future::ready(())
    }

    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("spook-host", "0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST)
    }
}

/// The loopback GM seat over streamable HTTP.
pub struct GmConnection {
    running: RunningService<RoleClient, Inbox>,
    /// Finishes when the host closes the agent door.
    watch: tokio::task::JoinHandle<()>,
    incoming: mpsc::UnboundedReceiver<Incoming>,
    buffered: VecDeque<Incoming>,
    next_id: u64,
    opened: String,
}

impl GmConnection {
    /// Dial `http://127.0.0.1:<port + 1>/mcp` and finish the handshake.
    pub async fn connect(addr: std::net::SocketAddr) -> Result<Self, Error> {
        dial(door_for(addr)?).await
    }

    /// Dial `http://127.0.0.1:<port>/mcp`. `door` is the agent door, not the game port.
    pub async fn connect_door(door: std::net::SocketAddr) -> Result<Self, Error> {
        dial(door).await
    }
}

fn door_for(addr: std::net::SocketAddr) -> Result<std::net::SocketAddr, Error> {
    crate::protocol::default_door(addr).ok_or_else(|| Error::Bad(addr.to_string()))
}

async fn dial(door: std::net::SocketAddr) -> Result<GmConnection, Error> {
    let url = format!("http://{door}/mcp");
    let (events, incoming) = mpsc::unbounded_channel();
    // A closed loopback seat ends the stream. The default client retries
    // that stream forever, so a dropped `spook run` never looks closed.
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());
    config.retry_config = Arc::new(NeverRetry::default());
    config.reinit_on_expired_session = false;
    let transport = SeatTransport {
        inner: StreamableHttpClientTransport::from_config(config),
        events: events.clone(),
        closed: false,
    };
    let running = match (Inbox {
        events: events.clone(),
    })
    .serve(transport)
    .await
    {
        Ok(running) => running,
        // The streamable client skips a JSON-RPC error on the initialize
        // event stream and reports the stream as closed. Read that refusal
        // from a fresh response body.
        Err(err @ ClientInitializeError::ConnectionClosed(_)) => {
            return Err(read_seat_refusal(&url)
                .await
                .unwrap_or_else(|| from_initialize(err)));
        }
        Err(err) => return Err(from_initialize(err)),
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
    Ok(GmConnection {
        opened: opened_of(&running),
        running,
        watch,
        incoming,
        buffered: VecDeque::new(),
        next_id: 2,
    })
}

impl GmConnection {
    /// The initialize result line, without a trailing newline.
    pub fn opened_line(&self) -> &str {
        &self.opened
    }

    pub async fn call(&mut self, name: &str, arguments: Value) -> Result<ToolCallResult, Error> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut outcome = call_tool(&self.running, name, arguments).await?;
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
            let _ = self.running.cancel().await;
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
    events: mpsc::UnboundedSender<Incoming>,
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
            let _ = self.events.send(Incoming::Eof);
        }
        message
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}

fn opened_of(running: &RunningService<RoleClient, Inbox>) -> String {
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
) -> Result<ToolCallResult, Error> {
    let params = match arguments {
        Value::Object(map) => CallToolRequestParams::new(name.to_string()).with_arguments(map),
        Value::Null => CallToolRequestParams::new(name.to_string()),
        other => return Err(Error::Bad(other.to_string())),
    };
    match running.call_tool(params).await {
        Ok(result) => Ok(tool_result(result)),
        Err(err) => Err(Error::Bad(err.to_string())),
    }
}

fn tool_result(result: CallToolResult) -> ToolCallResult {
    let body = result.structured_content.clone().unwrap_or(Value::Null);
    let refused = body.get("code").is_some();
    ToolCallResult {
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

async fn read_seat_refusal(url: &str) -> Option<Error> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    // Same version the seated client negotiated, so the probe cannot drift.
    // This initialize can sit a seat that freed after the first attempt.
    // The DELETE below gives that seat back.
    let version = ProtocolVersion::LATEST.as_str();
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{version}","capabilities":{{}},"clientInfo":{{"name":"spook-host","version":"0"}}}}}}"#
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
    let code = bracket_code(&text)?;
    Some(Error::Refused { code, raw: text })
}

fn from_initialize(err: ClientInitializeError) -> Error {
    if let Some(refused) = refusal_of(&err) {
        return refused;
    }
    Error::Io(std::io::Error::other(err.to_string()))
}

fn refusal_of(err: &ClientInitializeError) -> Option<Error> {
    match err {
        ClientInitializeError::JsonRpcError(data) => Some(refused_message(data.message.as_ref())),
        ClientInitializeError::LegacyFallbackFailed { discover, fallback } => {
            refusal_of(discover).or_else(|| refusal_of(fallback))
        }
        other => {
            let text = other.to_string();
            bracket_code(&text).map(|code| Error::Refused { code, raw: text })
        }
    }
}

fn refused_message(raw: &str) -> Error {
    let code = bracket_code(raw).unwrap_or_else(|| "SESSION_FULL".to_string());
    Error::Refused {
        code,
        raw: raw.to_string(),
    }
}

fn bracket_code(text: &str) -> Option<String> {
    let start = text.rfind('[')?;
    let end = text.rfind(']')?;
    if end <= start + 1 {
        return None;
    }
    let code = &text[start + 1..end];
    if code.is_empty() || code.contains([' ', '[']) {
        return None;
    }
    Some(code.to_string())
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
        Err(_) => Incoming::Bad(raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params_of(line: &str) -> Value {
        let value: Value = serde_json::from_str(line.trim_end()).unwrap();
        value.get("params").cloned().unwrap_or(Value::Null)
    }

    #[test]
    fn a_fixed_game_port_dials_the_next_port() {
        let game = std::net::SocketAddr::from(([127, 0, 0, 1], 9737));
        let door = door_for(game).ok().map(|door| format!("http://{door}/mcp"));
        assert_eq!(door.as_deref(), Some("http://127.0.0.1:9738/mcp"));
    }

    #[test]
    fn a_body_that_matches_no_known_type_is_bad() {
        let Incoming::Bad(line) = read_event(Some(&serde_json::json!({"kind": "nope"}))) else {
            panic!("malformed event was dropped");
        };
        assert!(line.contains("\"kind\":\"nope\""), "{line}");
    }

    #[test]
    fn a_say_event_keeps_the_speaker_and_the_text() {
        let line = String::from_utf8(crate::protocol::write_say_event(&crate::protocol::GmSay {
            seq: 4,
            from: "player:pc_rowan".to_string(),
            address: "table".to_string(),
            place: None,
            actor: None,
            recipients: vec!["pc_rowan".to_string()],
            text: "hello".to_string(),
        }))
        .unwrap();
        let Incoming::Event(event) = read_event(Some(&params_of(&line))) else {
            panic!("say");
        };
        let EventBody::Say(say) = event.body else {
            panic!("{:?}", event.body);
        };
        assert_eq!(say.from, "player:pc_rowan");
        assert_eq!(say.seq, 4);
        assert_eq!(say.text, "hello");
    }

    #[test]
    fn a_queued_event_keeps_the_actor_and_the_move() {
        let line = String::from_utf8(crate::protocol::write_queued_event(
            "id-1", "pc_rowan", "travel",
        ))
        .unwrap();
        let Incoming::Event(event) = read_event(Some(&params_of(&line))) else {
            panic!("queued");
        };
        assert_eq!(
            event.body,
            EventBody::Queued(crate::protocol::QueueRow {
                id: "id-1".to_string(),
                actor_id: "pc_rowan".to_string(),
                move_name: "travel".to_string(),
            })
        );
    }

    #[test]
    fn a_check_event_keeps_faces_and_drops_nothing_it_was_given() {
        let roll = crate::protocol::Roll {
            faces: vec![4, 2],
            stat: 1,
            total: 7,
            target: 0,
            outcome: crate::protocol::Outcome::Success,
        };
        let line = String::from_utf8(crate::protocol::write_check_event(&roll)).unwrap();
        let Incoming::Event(event) = read_event(Some(&params_of(&line))) else {
            panic!("check");
        };
        assert_eq!(event.body, EventBody::Check(roll));
    }
}
