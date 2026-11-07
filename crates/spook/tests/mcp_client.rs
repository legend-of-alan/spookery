//! A GM client of `/mcp` for tests: dials the MCP endpoint, calls tools, hears events.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, CustomNotification, Implementation,
    ProtocolVersion, Tool,
};
use rmcp::service::{NotificationContext, RoleClient, RunningService};
use rmcp::transport::common::client_side_sse::NeverRetry;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, Transport};
use rmcp::{ClientHandler, ServiceExt};
use serde_json::Value;
use spookery::protocol::{ServerBody, ServerMessage, ToolResult};
use tokio::sync::mpsc;

/// One item on the event channel: an event, the line for one that could not be read, or
/// `None` for the end of the seat.
type EventItem = Option<Result<ServerMessage, String>>;

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
            Implementation::new("spook-test-gm", "0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST)
    }
}

pub struct GmClient {
    service: RunningService<RoleClient, EventSender>,
    /// Finishes when the host closes the MCP endpoint.
    watch: tokio::task::JoinHandle<()>,
    rx: mpsc::UnboundedReceiver<EventItem>,
    buffered: VecDeque<EventItem>,
}

impl GmClient {
    /// Dial `http://<mcp_addr>/mcp`, finish the handshake, and wait for the event stream to
    /// take the seat: its hello. A refused seat is the refusal's sentence.
    pub async fn connect(mcp_addr: SocketAddr) -> Result<Self, String> {
        let url = format!("http://{mcp_addr}/mcp");
        let (tx, rx) = mpsc::unbounded_channel();
        // A closed loopback seat ends the stream. The default client retries
        // that stream forever, so a dropped `spook run` never looks closed.
        let mut config = StreamableHttpClientTransportConfig::with_uri(url);
        config.retry_config = Arc::new(NeverRetry::default());
        config.reinit_on_expired_session = false;
        let transport = ClientTransport {
            inner: StreamableHttpClientTransport::from_config(config),
            tx: tx.clone(),
            closed: false,
        };
        let sender = EventSender { tx: tx.clone() };
        let service = sender
            .serve(transport)
            .await
            .map_err(|err| err.to_string())?;
        // rmcp keeps no read on the MCP port once a tool call's stream
        // ends, so a killed `spook run` never looks closed. A plain connection
        // with a pending read sees that close.
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
        let mut gm = GmClient {
            service,
            watch,
            rx,
            buffered: VecDeque::new(),
        };
        match gm.recv().await {
            Some(Ok(hello)) if matches!(hello.body, ServerBody::Hello { .. }) => {
                gm.buffered.push_front(Some(Ok(hello)));
                Ok(gm)
            }
            Some(Ok(ServerMessage {
                body: ServerBody::Refused(refusal),
                ..
            })) => {
                gm.close().await;
                Err(refusal.to_string())
            }
            other => {
                gm.close().await;
                Err(format!("no hello on the event stream: {other:?}"))
            }
        }
    }

    /// The tools the seat lists.
    pub async fn list_tools(&self) -> Result<Vec<Tool>, String> {
        self.service
            .list_all_tools()
            .await
            .map_err(|err| err.to_string())
    }

    /// Send the command `command` as a tool call. The result is every message it sent this seat,
    /// its reply last.
    pub async fn call(
        &mut self,
        command: &str,
        args: &Value,
    ) -> Result<Vec<ServerMessage>, String> {
        let Value::Object(arguments) = args.clone() else {
            return Err(format!("{command} arguments: {args}"));
        };
        let params = CallToolRequestParams::new(command.to_string()).with_arguments(arguments);
        let result = self
            .service
            .call_tool(params)
            .await
            .map_err(|err| err.to_string())?;
        while let Ok(item) = self.rx.try_recv() {
            self.buffered.push_back(item);
        }
        let body = result.structured_content.unwrap_or(Value::Null);
        serde_json::from_value::<ToolResult>(body.clone())
            .map(|result| result.messages)
            .map_err(|_| format!("{command} result: {body}"))
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

    /// The next event, or the line for one that could not be read. `None` when the seat ends.
    pub async fn recv(&mut self) -> EventItem {
        if let Some(item) = self.buffered.pop_front() {
            return item;
        }
        self.rx.recv().await.flatten()
    }

    /// Buffered events, then any the reader has already queued. An end of the seat among
    /// them is dropped.
    pub fn drain(&mut self) -> Vec<Result<ServerMessage, String>> {
        let mut out: Vec<Result<ServerMessage, String>> =
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

/// The streamable HTTP transport, plus one end-of-seat notice: `receive` returning nothing
/// sends `None` on the event channel, so `GmClient::recv` returns `None`.
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

/// One `spook/event` notification's params, one `ServerMessage`, or the notification's line.
fn parse_event(params: Option<Value>) -> Result<ServerMessage, String> {
    let params = params.unwrap_or(Value::Null);
    serde_json::from_value(params.clone()).map_err(|_| notification_line(&params))
}

/// One `spook/event` notification as its JSON-RPC line.
pub fn event_line(event: &ServerMessage) -> String {
    notification_line(&serde_json::json!(event))
}

fn notification_line(params: &Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "method": "spook/event", "params": params }).to_string()
}

/// One MCP session over plain HTTP, for what the rmcp client hides: the raw replies, and
/// event streams opened by hand.
pub struct RawSession {
    port: u16,
    id: String,
    version: String,
    /// The `initialize` reply.
    pub opened: String,
    /// The event streams this session opened, kept open.
    streams: Vec<std::net::TcpStream>,
}

impl RawSession {
    /// `initialize`, then `notifications/initialized`. That opens a session and takes no seat.
    pub fn open(port: u16) -> Self {
        let initialize = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": ProtocolVersion::LATEST.as_str(),
                "capabilities": {},
                "clientInfo": { "name": "raw", "version": "0" }
            }
        });
        let opened = send_http(
            port,
            "POST",
            None,
            ProtocolVersion::LATEST.as_str(),
            &initialize,
        );
        let id = opened
            .lines()
            .take_while(|line| !line.is_empty())
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("mcp-session-id")
                    .then(|| value.trim().to_string())
            })
            .unwrap_or_else(|| panic!("no session\n{opened}"));
        let version = opened
            .split_once("\"protocolVersion\":\"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(version, _)| version.to_string())
            .unwrap_or_else(|| panic!("no protocol version\n{opened}"));
        let session = RawSession {
            port,
            id,
            version,
            opened,
            streams: Vec::new(),
        };
        let initialized = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let reply = session.post(&initialized);
        assert!(reply.contains(" 202 "), "initialized was refused\n{reply}");
        session
    }

    /// Opens an event stream, which takes the GM seat, and keeps it open. Returns what the
    /// stream sent through its first event.
    pub fn open_event_stream(&mut self) -> String {
        use std::io::{Read, Write};

        let (port, id, version) = (self.port, &self.id, &self.version);
        let request = format!(
            "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nmcp-session-id: {id}\r\nMCP-Protocol-Version: {version}\r\nAccept: text/event-stream\r\n\r\n"
        );
        let mut stream = connect_http(port);
        stream
            .write_all(request.as_bytes())
            .unwrap_or_else(|err| panic!("write GET: {err}"));
        let mut raw = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let read = stream
                .read(&mut buf)
                .unwrap_or_else(|err| panic!("read GET: {err}\n{}", String::from_utf8_lossy(&raw)));
            raw.extend_from_slice(&buf[..read]);
            let text = String::from_utf8_lossy(&raw);
            let has_event = text
                .split_once("spook/event")
                .is_some_and(|(_, rest)| rest.contains("\n\n"));
            if read == 0 || has_event {
                break;
            }
        }
        self.streams.push(stream);
        String::from_utf8_lossy(&raw).into_owned()
    }

    /// Calls the tool `command` with no arguments. Returns the raw reply.
    pub fn call(&self, command: &str) -> String {
        let call = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": command, "arguments": {} }
        });
        self.post(&call)
    }

    fn post(&self, body: &Value) -> String {
        send_http(self.port, "POST", Some(&self.id), &self.version, body)
    }
}

/// The first `spook/event` notification in a raw event stream.
pub fn read_event(raw: &str) -> Value {
    raw.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .find(|message| message["method"] == "spook/event")
        .unwrap_or_else(|| panic!("no spook/event\n{raw}"))
}

/// One HTTP/1.1 request on its own connection. Returns the raw reply.
fn send_http(
    port: u16,
    method: &str,
    session: Option<&str>,
    version: &str,
    body: &Value,
) -> String {
    use std::io::{Read, Write};

    let body = body.to_string();
    let session = session
        .map(|id| format!("mcp-session-id: {id}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{session}MCP-Protocol-Version: {version}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = connect_http(port);
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|err| panic!("write HTTP: {err}"));
    let mut raw = String::new();
    let _ = stream.read_to_string(&mut raw);
    raw
}

fn connect_http(port: u16) -> std::net::TcpStream {
    let stream = std::net::TcpStream::connect(("127.0.0.1", port))
        .unwrap_or_else(|err| panic!("connect HTTP: {err}"));
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    stream
}
