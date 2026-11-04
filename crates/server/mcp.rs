//! The MCP endpoint: the GM seat over MCP streamable HTTP.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{ready, Context, Poll};

use futures_core::Stream;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientJsonRpcMessage,
    CustomNotification, GetExtensions, InitializeRequestParams, InitializeResult, ListToolsResult,
    PaginatedRequestParams, ServerConfig, ServerJsonRpcMessage, ServerNotification, Tool,
    ToolsCapability,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::{
    LocalSessionManager, LocalSessionManagerError, SessionError,
};
use rmcp::transport::streamable_http_server::session::{
    ServerSseMessage, SessionId, SessionManager,
};
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, ServerHandler};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use super::gm;
use super::limits::GM_BODY_CAP;
use super::seat::{frame_queue, Conn, FrameReceiver};
use super::table::{report_hangup, Request};
use crate::protocol::{ClientBody, ClientMessage, Seat, ToolResult, WORDS};

pub(super) async fn serve(
    listener: tokio::net::TcpListener,
    tx: Sender<Request>,
    next_conn: Arc<AtomicU64>,
) {
    let (hangup_tx, mut hangups) = unbounded_channel();
    let sessions = Arc::new(SeatSessions::new(tx.clone(), next_conn, hangup_tx));
    let closer = SessionGuard(Arc::clone(&sessions));
    let service = StreamableHttpService::new(
        move || Ok(Handler::new(tx.clone())),
        sessions,
        http_config(),
    );
    let app = axum::Router::new().route_service("/mcp", service);
    // Hangups are closed here, so the loop ends with `serve`.
    tokio::select! {
        _ = axum::serve(listener, app) => {}
        () = async {
            while let Some(id) = hangups.recv().await {
                let _ = closer.0.close_session(&id).await;
            }
        } => {}
    }
}

/// Ends rmcp's sessions when `serve` stops. Each session's task holds the session manager,
/// so a session left open would keep its task, and the table's sender, alive. If a session
/// is opening or closing at that moment the lock is held and nothing is cleared, so the
/// session tasks, and the sender, outlive `serve`.
struct SessionGuard(Arc<SeatSessions>);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.0.inner.sessions.try_write() {
            sessions.clear();
        }
    }
}

fn http_config() -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["127.0.0.1"])
        .with_allowed_origins(["http://127.0.0.1:*"]);
    // HTTP POST bodies on the MCP endpoint stop at this cap.
    config.max_request_body_bytes = GM_BODY_CAP;
    config
}

/// rmcp's local sessions. A session's `GET /mcp` event stream takes the GM seat, and
/// closing the session answers only after the table has freed it. A session ends on
/// DELETE or when the GM's event stream hangs up, never on an idle timer.
struct SeatSessions {
    inner: LocalSessionManager,
    tx: Sender<Request>,
    next_conn: Arc<AtomicU64>,
    conns: Mutex<HashMap<SessionId, Conn>>,
    hangup_tx: UnboundedSender<SessionId>,
}

impl SeatSessions {
    fn new(
        tx: Sender<Request>,
        next_conn: Arc<AtomicU64>,
        hangup_tx: UnboundedSender<SessionId>,
    ) -> Self {
        let mut inner = LocalSessionManager::default();
        inner.session_config.keep_alive = None;
        Self {
            inner,
            tx,
            next_conn,
            conns: Mutex::new(HashMap::new()),
            hangup_tx,
        }
    }
}

/// The GM's event stream: each frame the table queues for the seat, as one `spook/event`
/// notification over SSE. It owns the seat, so when it is dropped the session is closed.
/// A refused stream carries only its refusal and owns nothing.
struct EventStream {
    frames_rx: FrameReceiver,
    /// The session whose seat this stream holds, if it holds one.
    seated: Option<(SessionId, UnboundedSender<SessionId>)>,
}

impl Stream for EventStream {
    type Item = ServerSseMessage;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(frame) = ready!(self.frames_rx.poll_frame(cx)) else {
            return Poll::Ready(None);
        };
        let body = serde_json::from_slice(&frame).expect("the table queues JSON");
        let note = CustomNotification::new("spook/event", Some(body));
        let message =
            ServerJsonRpcMessage::notification(ServerNotification::CustomNotification(note));
        Poll::Ready(Some(ServerSseMessage::from_message(message)))
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        if let Some((id, hangup_tx)) = self.seated.take() {
            let _ = hangup_tx.send(id);
        }
    }
}

impl SessionManager for SeatSessions {
    type Error = LocalSessionManagerError;
    type Transport = <LocalSessionManager as SessionManager>::Transport;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        self.inner.create_session().await
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        mut message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        if let ClientJsonRpcMessage::Request(request) = &mut message {
            let conn = Conn::new(&self.next_conn);
            self.conns
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .insert(id.clone(), conn);
            // `Handler` reads the id back during initialize.
            request.request.extensions_mut().insert(conn);
        }
        self.inner.initialize_session(id, message).await
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        self.inner.has_session(id).await
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        // Remove first, so the worker's second call after a DELETE is a no-op.
        let conn = self
            .conns
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(id);
        let closed = self.inner.close_session(id).await;
        if let Some(conn) = conn {
            report_hangup(&self.tx, conn).await;
        }
        closed
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        self.inner.create_stream(id, message).await
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        self.inner.accept_message(id, message).await
    }

    /// Takes the GM seat for the session. A refused seat is a stream of one refusal, and
    /// it closes nothing.
    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let conn = self
            .conns
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(id)
            .copied()
            .ok_or_else(|| LocalSessionManagerError::SessionNotFound(id.clone()))?;
        let (frames_tx, frames_rx) = frame_queue();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Request::Open {
                conn,
                seat: Seat::Gm,
                frames_tx,
                reply: reply_tx,
            })
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        match reply_rx
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?
        {
            Ok(()) => Ok(EventStream {
                frames_rx,
                seated: Some((id.clone(), self.hangup_tx.clone())),
            }),
            Err(refused) => {
                let (refusal_tx, frames_rx) = frame_queue();
                refusal_tx.send(refused.to_text().into_bytes());
                Ok(EventStream {
                    frames_rx,
                    seated: None,
                })
            }
        }
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        self.inner.resume(id, last_event_id).await
    }
}

/// The error a tool call gets once the table has stopped.
fn table_closed() -> McpError {
    McpError::internal_error("table closed", None)
}

struct Handler {
    tx: Sender<Request>,
    conn: OnceLock<Conn>,
}

impl Handler {
    fn new(tx: Sender<Request>) -> Self {
        Self {
            tx,
            conn: OnceLock::new(),
        }
    }
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::default();
        info.capabilities.tools = Some(ToolsCapability::default());
        info
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(list_gm_tools())))
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        let conn = context
            .extensions
            .get::<Conn>()
            .copied()
            .ok_or_else(|| McpError::internal_error("no seat", None))?;
        let _ = self.conn.set(conn);
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    /// The tool is the word and its arguments are the word's `args`. The JSON-RPC id is the
    /// message `id`, so the reply's `reply_to` names this call.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(&conn) = self.conn.get() else {
            return Err(McpError::internal_error("no seat", None));
        };
        let message = ClientMessage {
            id: Some(context.id.to_string()),
            seq: None,
            reply_to: None,
            body: ClientBody {
                word: request.name.into_owned(),
                args: request.arguments.unwrap_or_default(),
            },
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Request::ToolCall {
                conn,
                message,
                reply: reply_tx,
            })
            .map_err(|_| table_closed())?;
        let messages = reply_rx.await.map_err(|_| table_closed())?;
        let result = serde_json::to_value(ToolResult { messages }).expect("tool result");
        Ok(CallToolResult::structured(result).into())
    }
}

/// One tool per GM word, in the word table's order and named after the word. Its input schema
/// is the word's `args`.
fn list_gm_tools() -> Vec<Tool> {
    WORDS
        .iter()
        .filter(|word| word.seat == Seat::Gm)
        .filter_map(|word| {
            let (about, schema) = gm::describe_gm_word(word.name)?;
            Some(Tool::new(
                word.name,
                format!("{about} {SEAT_NOTES}"),
                schema,
            ))
        })
        .collect()
}

const SEAT_NOTES: &str = "The result lists every message the word sent this seat, its reply last. Player moves stay queued until commit. say does not tick. Other messages arrive as spook/event notifications on the GET /mcp event stream, which holds the seat.";

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
    use std::sync::{mpsc, Arc};

    use rmcp::model::ProtocolVersion;
    use serde_json::Value;

    use super::serve;
    use crate::seat::Conn;
    use crate::table::Request;

    /// A stand-in for the table thread with one GM seat. It refuses a second
    /// claim while the seat is held, like the real table, and sends a seated
    /// GM a hello. On a fault it tells the test, then holds the seat until the
    /// test lets the release through. The thread ends when every sender to it
    /// is gone.
    fn spawn_fake_table(
        rx: mpsc::Receiver<Request>,
        fault_seen: tokio::sync::mpsc::UnboundedSender<()>,
        let_go: mpsc::Receiver<()>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut seat: Option<Conn> = None;
            let mut kept = Vec::new();
            for msg in rx {
                match msg {
                    Request::Open {
                        conn,
                        frames_tx: out,
                        reply,
                        ..
                    } => {
                        if seat.is_some() {
                            let refused = crate::protocol::ServerMessage {
                                id: None,
                                seq: Some(1),
                                reply_to: None,
                                body: crate::protocol::ServerBody::Refused(
                                    spookery_engine::Reason::TableFull.into(),
                                ),
                            };
                            let _ = reply.send(Err(Box::new(refused)));
                        } else {
                            seat = Some(conn);
                            out.send(br#"{"body":{"kind":"hello"}}"#.to_vec());
                            kept.push(out);
                            let _ = reply.send(Ok(()));
                        }
                    }
                    Request::Hangup { conn, ack, .. } => {
                        let _ = fault_seen.send(());
                        if let_go.recv().is_err() {
                            return;
                        }
                        if seat == Some(conn) {
                            seat = None;
                        }
                        let _ = ack.send(());
                    }
                    _ => {}
                }
            }
        })
    }

    fn initialize_request() -> Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": ProtocolVersion::LATEST.as_str(),
                "capabilities": {},
                "clientInfo": { "name": "seat-release", "version": "0" }
            }
        })
    }

    /// One HTTP/1.1 request on its own connection. Returns the raw reply.
    async fn send_http(
        port: u16,
        method: &str,
        session: Option<&str>,
        version: &str,
        body: &str,
    ) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let session = session
            .map(|id| format!("mcp-session-id: {id}\r\n"))
            .unwrap_or_default();
        let request = format!(
            "{method} /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{session}MCP-Protocol-Version: {version}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap_or_else(|err| panic!("connect: {err}"));
        stream
            .write_all(request.as_bytes())
            .await
            .unwrap_or_else(|err| panic!("write: {err}"));
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .unwrap_or_else(|err| panic!("read: {err}"));
        String::from_utf8_lossy(&raw).into_owned()
    }

    /// Opens a session. Returns its id and the raw reply.
    async fn open(port: u16) -> (Option<String>, String) {
        let raw = send_http(
            port,
            "POST",
            None,
            ProtocolVersion::LATEST.as_str(),
            &initialize_request().to_string(),
        )
        .await;
        let session = raw
            .lines()
            .take_while(|line| !line.is_empty())
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("mcp-session-id")
                    .then(|| value.trim().to_string())
            });
        (session, raw)
    }

    /// Opens the session's event stream, which takes the GM seat, and leaves it
    /// open. Returns the socket and the stream through its first event.
    /// Dropping the socket hangs up, like a GM process that dies.
    async fn open_event_stream(
        port: u16,
        session: &str,
        version: &str,
    ) -> (tokio::net::TcpStream, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let request = format!(
            "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nmcp-session-id: {session}\r\nMCP-Protocol-Version: {version}\r\nAccept: text/event-stream\r\n\r\n"
        );
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap_or_else(|err| panic!("connect: {err}"));
        stream
            .write_all(request.as_bytes())
            .await
            .unwrap_or_else(|err| panic!("write: {err}"));
        let mut raw = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            let read = stream
                .read(&mut buf)
                .await
                .unwrap_or_else(|err| panic!("read: {err}"));
            raw.extend_from_slice(&buf[..read]);
            let text = String::from_utf8_lossy(&raw);
            let has_event = text
                .split_once("spook/event")
                .is_some_and(|(_, rest)| rest.contains("\n\n"));
            if read == 0 || has_event {
                break;
            }
        }
        let head = String::from_utf8_lossy(&raw).into_owned();
        assert!(
            head.contains(" 200 "),
            "the event stream did not open\n{head}"
        );
        (stream, head)
    }

    fn read_negotiated_version(body: &str) -> String {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .chain(std::iter::once(body))
            .filter_map(|text| serde_json::from_str::<Value>(text.trim()).ok())
            .find_map(|value| {
                value["result"]["protocolVersion"]
                    .as_str()
                    .map(str::to_string)
            })
            .unwrap_or_else(|| panic!("no protocol version\n{body}"))
    }

    async fn bind_mcp() -> (
        u16,
        tokio::sync::mpsc::UnboundedReceiver<()>,
        mpsc::Sender<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|err| panic!("bind: {err}"));
        let port = listener
            .local_addr()
            .unwrap_or_else(|err| panic!("addr: {err}"))
            .port();
        let (tx, rx) = mpsc::channel();
        let (fault_tx, fault_seen) = tokio::sync::mpsc::unbounded_channel();
        let (let_go, held) = mpsc::channel();
        spawn_fake_table(rx, fault_tx, held);
        tokio::spawn(serve(listener, tx, Arc::new(AtomicU64::new(1))));
        (port, fault_seen, let_go)
    }

    /// Opens a session and finishes its handshake. It holds no seat yet.
    async fn open_session(port: u16) -> (String, String) {
        let (session, body) = open(port).await;
        let session = session.unwrap_or_else(|| panic!("no session\n{body}"));
        let version = read_negotiated_version(&body);
        let initialized = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let reply = send_http(
            port,
            "POST",
            Some(&session),
            &version,
            &initialized.to_string(),
        )
        .await;
        assert!(reply.contains(" 202 "), "initialized was refused\n{reply}");
        (session, version)
    }

    /// A session whose event stream holds the GM seat.
    async fn sit(port: u16) -> (String, String, tokio::net::TcpStream) {
        let (session, version) = open_session(port).await;
        let (events, head) = open_event_stream(port, &session, &version).await;
        assert!(head.contains("\"hello\""), "the GM did not sit\n{head}");
        (session, version, events)
    }

    // A GM that dies without DELETE hangs up its event stream. That frees
    // the seat, so the next GM sits at once (issue #313).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gm_that_hangs_up_frees_the_seat() {
        let (port, mut fault_seen, let_go) = bind_mcp().await;
        let (_session, _version, events) = sit(port).await;

        drop(events);
        assert!(
            fault_seen.recv().await.is_some(),
            "the seat was never released"
        );
        let_go
            .send(())
            .unwrap_or_else(|err| panic!("let go: {err}"));

        sit(port).await;
    }

    // A GM that holds its event stream open but sends nothing keeps the seat
    // past rmcp's old 300 s idle window (issue #313).
    #[tokio::test]
    async fn a_quiet_gm_keeps_the_seat() {
        let (port, mut fault_seen, _let_go) = bind_mcp().await;
        let (session, version, _events) = sit(port).await;

        tokio::time::pause();
        tokio::time::sleep(std::time::Duration::from_secs(301)).await;
        tokio::time::resume();

        let list = serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let reply = send_http(port, "POST", Some(&session), &version, &list.to_string()).await;
        assert!(
            reply.contains("\"tools\""),
            "the quiet GM lost its session\n{reply}"
        );
        assert!(fault_seen.try_recv().is_err(), "the quiet GM lost the seat");
    }

    // A second event stream on a seated session is refused, and closing it
    // leaves the first stream and its seat alone (issue #459).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_event_stream_closes_nothing() {
        let (port, mut fault_seen, _let_go) = bind_mcp().await;
        let (session, version, _events) = sit(port).await;

        let (second, head) = open_event_stream(port, &session, &version).await;
        assert!(head.contains("TABLE_FULL"), "a second stream sat\n{head}");
        drop(second);

        let list = serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let reply = send_http(port, "POST", Some(&session), &version, &list.to_string()).await;
        assert!(
            reply.contains("\"tools\""),
            "the GM lost its session\n{reply}"
        );
        assert!(fault_seen.try_recv().is_err(), "the GM lost the seat");
    }

    // End to end over HTTP: the handler's drop reaches the table thread, the
    // DELETE reply waits for the release, and a GM that reconnects as soon as
    // DELETE answers is not told the table is full (issue #214).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gm_can_sit_again_as_soon_as_delete_answers() {
        let (port, mut fault_seen, let_go) = bind_mcp().await;
        let (session, version, _events) = sit(port).await;

        let mut delete =
            tokio::spawn(
                async move { send_http(port, "DELETE", Some(&session), &version, "").await },
            );

        // The release reaches the table thread, which holds the seat for now.
        tokio::select! {
            biased;
            reply = &mut delete => {
                panic!("DELETE answered before the seat release was confirmed\n{reply:?}");
            }
            seen = fault_seen.recv() => {
                assert!(seen.is_some(), "the seat was never released");
            }
        }
        assert!(
            !delete.is_finished(),
            "DELETE answered while the table thread still held the seat"
        );

        // Let the release through. Only now may DELETE answer.
        let_go
            .send(())
            .unwrap_or_else(|err| panic!("let go: {err}"));
        let reply = delete
            .await
            .unwrap_or_else(|err| panic!("DELETE task: {err}"));
        let status = reply.lines().next().unwrap_or("");
        assert!(
            status.contains(" 200 ") || status.contains(" 202 ") || status.contains(" 204 "),
            "DELETE did not answer with success\n{reply}"
        );

        // A GM that reconnects as soon as DELETE answers finds the seat free.
        sit(port).await;
    }

    // Once `serve` stops, nothing it started still holds the table's
    // channel, so the table thread sees it close (issue #459).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nothing_outlives_serve() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|err| panic!("bind: {err}"));
        let port = listener
            .local_addr()
            .unwrap_or_else(|err| panic!("addr: {err}"))
            .port();
        let (tx, rx) = mpsc::channel();
        let (fault_tx, _fault_seen) = tokio::sync::mpsc::unbounded_channel();
        let (let_go, held) = mpsc::channel();
        // The session's close reports a hangup. Let it through.
        let_go
            .send(())
            .unwrap_or_else(|err| panic!("let go: {err}"));
        let table = spawn_fake_table(rx, fault_tx, held);
        let served = tokio::spawn(serve(listener, tx, Arc::new(AtomicU64::new(1))));
        open_session(port).await;

        served.abort();
        let closed = tokio::task::spawn_blocking(move || table.join());
        tokio::time::timeout(std::time::Duration::from_secs(10), closed)
            .await
            .unwrap_or_else(|_| panic!("a task from serve still holds the table's channel"))
            .unwrap_or_else(|err| panic!("join: {err}"))
            .unwrap_or_else(|_| panic!("the table thread panicked"));
    }
}
