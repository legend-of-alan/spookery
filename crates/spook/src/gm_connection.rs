//! The client that sits the GM seat over the agent door.

use std::collections::VecDeque;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, CustomNotification,
    Implementation, ProtocolVersion,
};
use rmcp::service::{ClientInitializeError, NotificationContext, RoleClient};
use rmcp::transport::common::client_side_sse::NeverRetry;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, Transport};
use rmcp::{ClientHandler, ServiceExt};
use serde_json::Value;

/// A GM-seat failure. The display text is the seat's message or the I/O error.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Closed,
    Bad(String),
    Refused { code: String, raw: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Closed => f.write_str("seat closed"),
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

enum Command {
    Call {
        name: String,
        arguments: Value,
        reply: mpsc::Sender<Result<ToolCallResult, Error>>,
    },
    Shutdown,
}

struct Inbox {
    events: mpsc::Sender<Incoming>,
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
    commands: tokio::sync::mpsc::Sender<Command>,
    incoming: mpsc::Receiver<Incoming>,
    buffered: VecDeque<Incoming>,
    next_id: u64,
    opened: String,
    worker: Option<JoinHandle<()>>,
}

impl GmConnection {
    /// Dial `http://127.0.0.1:<port + 1>/mcp` and finish the handshake.
    pub fn connect(addr: &str) -> Result<Self, Error> {
        dial(next_door_addr(addr)?)
    }

    /// Dial `http://127.0.0.1:<port>/mcp`. `addr` is the agent door, not the game port.
    pub fn connect_door(addr: &str) -> Result<Self, Error> {
        dial(door_addr(addr)?)
    }
}

fn dial(door: std::net::SocketAddr) -> Result<GmConnection, Error> {
    let url = format!("http://{door}/mcp");
    let (commands, commands_rx) = tokio::sync::mpsc::channel(4);
    let (events, incoming) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let worker = thread::spawn(move || worker(door, url, commands_rx, events, ready_tx));
    let (opened, worker) = park(move || match ready_rx.recv() {
        Ok(Ok(opened)) => Ok((opened, worker)),
        Ok(Err(err)) => {
            let _ = worker.join();
            Err(err)
        }
        Err(_) => {
            let _ = worker.join();
            Err(Error::Closed)
        }
    })?;
    Ok(GmConnection {
        commands,
        incoming,
        buffered: VecDeque::new(),
        next_id: 2,
        opened,
        worker: Some(worker),
    })
}

impl GmConnection {
    /// The initialize result line, without a trailing newline.
    pub fn opened_line(&self) -> &str {
        &self.opened
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> Result<ToolCallResult, Error> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let (reply_tx, reply_rx) = mpsc::channel();
        let commands = self.commands.clone();
        park(move || {
            commands
                .blocking_send(Command::Call {
                    name: name.to_string(),
                    arguments,
                    reply: reply_tx,
                })
                .map_err(|_| Error::Closed)
        })?;
        let mut outcome = park(move || reply_rx.recv().unwrap_or(Err(Error::Closed)))?;
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

    pub fn next_incoming(&mut self) -> Incoming {
        if let Some(item) = self.buffered.pop_front() {
            return item;
        }
        self.incoming.recv().unwrap_or(Incoming::Eof)
    }

    pub fn poll(&mut self, timeout: Duration) -> Option<Incoming> {
        if let Some(item) = self.buffered.pop_front() {
            return Some(item);
        }
        match self.incoming.recv_timeout(timeout) {
            Ok(item) => Some(item),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(Incoming::Eof),
        }
    }

    /// Buffered lines, then anything the reader has already queued.
    pub fn drain_ready(&mut self) -> Vec<Incoming> {
        let mut out: Vec<Incoming> = self.buffered.drain(..).collect();
        while let Ok(item) = self.incoming.try_recv() {
            out.push(item);
        }
        out
    }
}

impl Drop for GmConnection {
    fn drop(&mut self) {
        let commands = self.commands.clone();
        let worker = self.worker.take();
        park(move || {
            let _ = commands.blocking_send(Command::Shutdown);
            if let Some(worker) = worker {
                let _ = worker.join();
            }
        });
    }
}

/// `blocking_send` panics on a runtime thread. Cucumber steps run on one.
fn park<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

/// The streamable HTTP transport, plus one end-of-seat notice.
///
/// `receive` returning nothing is the seat going away. The TCP client turned
/// that into `Incoming::Eof`. Callers waiting on the next event use the same
/// signal to leave.
struct SeatTransport<T> {
    inner: T,
    events: mpsc::Sender<Incoming>,
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

fn worker(
    door: std::net::SocketAddr,
    url: String,
    mut commands: tokio::sync::mpsc::Receiver<Command>,
    events: mpsc::Sender<Incoming>,
    ready: mpsc::Sender<Result<String, Error>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            let _ = ready.send(Err(Error::Io(err)));
            return;
        }
    };
    runtime.block_on(async move {
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
        let watch_events = events.clone();
        let running = match (Inbox { events }).serve(transport).await {
            Ok(running) => running,
            Err(err) => {
                // The streamable client skips a JSON-RPC error on the
                // initialize event stream and reports the stream as closed.
                // Read that refusal from a fresh response body.
                let mapped = if matches!(err, ClientInitializeError::ConnectionClosed(_)) {
                    match read_seat_refusal(&url).await {
                        Some(refused) => refused,
                        None => from_initialize(err),
                    }
                } else {
                    from_initialize(err)
                };
                let _ = ready.send(Err(mapped));
                return;
            }
        };
        let _ = ready.send(Ok(opened_of(&running)));
        // rmcp keeps no read on the agent port once a tool call's stream
        // ends, so a killed `spook run` never looks closed. A plain connection
        // with a pending read sees that close the way the TCP client did.
        tokio::spawn(async move {
            let Ok(mut socket) = tokio::net::TcpStream::connect(door).await else {
                let _ = watch_events.send(Incoming::Eof);
                return;
            };
            let mut buf = [0u8; 1];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
            let _ = watch_events.send(Incoming::Eof);
        });
        while let Some(command) = commands.recv().await {
            match command {
                Command::Shutdown => break,
                Command::Call {
                    name,
                    arguments,
                    reply,
                } => {
                    let _ = reply.send(call_tool(&running, &name, arguments).await);
                }
            }
        }
        let _ = running.cancel().await;
    });
}

fn opened_of(running: &rmcp::service::RunningService<RoleClient, Inbox>) -> String {
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
    running: &rmcp::service::RunningService<RoleClient, Inbox>,
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

fn loopback_port(addr: &str) -> Result<u16, Error> {
    addr.strip_prefix("127.0.0.1:")
        .and_then(|text| text.parse::<u16>().ok())
        .ok_or_else(|| Error::Bad(addr.to_string()))
}

fn next_door_addr(addr: &str) -> Result<std::net::SocketAddr, Error> {
    let port = loopback_port(addr)?;
    let http = port
        .checked_add(1)
        .ok_or_else(|| Error::Bad(addr.to_string()))?;
    Ok(std::net::SocketAddr::from(([127, 0, 0, 1], http)))
}

fn door_addr(addr: &str) -> Result<std::net::SocketAddr, Error> {
    let port = loopback_port(addr)?;
    Ok(std::net::SocketAddr::from(([127, 0, 0, 1], port)))
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

    #[test]
    fn the_agent_door_is_the_next_port() {
        assert_eq!(
            next_door_addr("127.0.0.1:9737").unwrap().to_string(),
            "127.0.0.1:9738"
        );
        assert!(next_door_addr("127.0.0.1:65535").is_err());
        assert!(next_door_addr("10.0.0.1:9").is_err());
        assert_eq!(
            door_addr("127.0.0.1:9801").unwrap().to_string(),
            "127.0.0.1:9801"
        );
    }

    fn params_of(line: &str) -> Value {
        let value: Value = serde_json::from_str(line.trim_end()).unwrap();
        value.get("params").cloned().unwrap_or(Value::Null)
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
