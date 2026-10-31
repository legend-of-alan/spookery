//! The agent door: the GM seat over MCP streamable HTTP.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};

use futures_core::Stream;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientJsonRpcMessage,
    CustomNotification, GetExtensions, InitializeRequestParams, InitializeResult, ListToolsResult,
    PaginatedRequestParams, ServerConfig, ServerJsonRpcMessage, ServerNotification, Tool,
    ToolsCapability,
};
use rmcp::service::{Peer, RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::{
    LocalSessionManager, LocalSessionManagerError,
};
use rmcp::transport::streamable_http_server::session::{
    ServerSseMessage, SessionId, SessionManager,
};
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, ServerHandler};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use super::limits::GM_BODY_CAP;
use super::seat::{seat, write_frames, Conn};
use super::table::{report_hangup, Request};
use super::tools;

pub(super) async fn serve(
    listener: tokio::net::TcpListener,
    tx: Sender<Request>,
    next_conn: Arc<AtomicU64>,
) {
    let (hangup_tx, mut hang_ups) = unbounded_channel();
    let sessions = Arc::new(SeatSessions::new(tx.clone(), next_conn, hangup_tx));
    let closer = Arc::clone(&sessions);
    tokio::spawn(async move {
        while let Some(id) = hang_ups.recv().await {
            let _ = closer.close_session(&id).await;
        }
    });
    let service = StreamableHttpService::new(
        move || Ok(Handler::new(tx.clone())),
        sessions,
        http_config(),
    );
    let app = axum::Router::new().route_service("/mcp", service);
    let _ = axum::serve(listener, app).await;
}

fn http_config() -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["127.0.0.1"])
        .with_allowed_origins(["http://127.0.0.1:*"]);
    // HTTP POST bodies on the agent door stop at this cap.
    config.max_request_body_bytes = GM_BODY_CAP;
    config
}

/// rmcp's local sessions. Closing one answers only after the table has
/// freed that session's GM seat. A session ends on DELETE or when the GM's
/// event stream hangs up, never on an idle timer.
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

/// Carries `spook/event` frames from the table to the GM's MCP client over SSE.
/// The GM's event stream. The connection under it lives as long as the GM,
/// so when it is dropped the session is closed.
struct EventStream<S> {
    inner: S,
    id: SessionId,
    hangup_tx: UnboundedSender<SessionId>,
}

impl<S: Stream + Unpin> Stream for EventStream<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<S> Drop for EventStream<S> {
    fn drop(&mut self) {
        let _ = self.hangup_tx.send(self.id.clone());
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

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let inner = self.inner.create_standalone_stream(id).await?;
        Ok(EventStream {
            inner,
            id: id.clone(),
            hangup_tx: self.hangup_tx.clone(),
        })
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
        let tools = tools::TOOLS
            .iter()
            .map(|tool| Tool::new(tool.name, tool.description(), Arc::new((tool.schema)())))
            .collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
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
        self.seat_gm(conn, &context).await?;
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(&conn) = self.conn.get() else {
            return Err(McpError::internal_error("no seat", None));
        };
        let tx = self.tx.clone();
        let name = request.name.into_owned();
        let arguments = request.arguments;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(Request::Tool {
            conn,
            name,
            arguments,
            reply: reply_tx,
        })
        .map_err(|_| table_closed())?;
        let value = reply_rx.await.map_err(|_| table_closed())?;
        Ok(CallToolResult::structured(value).into())
    }
}

impl Handler {
    async fn seat_gm(
        &self,
        conn: Conn,
        context: &RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        let (out, inbound) = seat();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Request::SeatGm {
                conn,
                seat: out,
                reply: reply_tx,
            })
            .map_err(|_| table_closed())?;
        let reply = reply_rx.await.map_err(|_| table_closed())?;
        match reply {
            Ok(()) => {
                let peer = context.peer.clone();
                // One writer drains this queue, so two tool calls cannot pass each other.
                tokio::spawn(write_frames(
                    async move |frame: &[u8]| {
                        send_event(&peer, frame).await;
                        true
                    },
                    inbound,
                ));
                Ok(())
            }
            Err(refusal) => Err(refusal.mcp_error()),
        }
    }
}

async fn send_event(peer: &Peer<RoleServer>, frame: &[u8]) {
    let Ok(body) = serde_json::from_slice(frame) else {
        return;
    };
    let note = CustomNotification::new("spook/event", Some(body));
    let _ = peer
        .send_notification(ServerNotification::CustomNotification(note))
        .await;
}

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
    /// claim while the seat is held, like the real table. On a fault it tells
    /// the test, then holds the seat until the test lets the release through.
    fn spawn_fake_table(
        rx: mpsc::Receiver<Request>,
        fault_seen: tokio::sync::mpsc::UnboundedSender<()>,
        let_go: mpsc::Receiver<()>,
    ) {
        std::thread::spawn(move || {
            let mut seat: Option<Conn> = None;
            let mut kept = Vec::new();
            for msg in rx {
                match msg {
                    Request::SeatGm {
                        conn,
                        seat: out,
                        reply,
                    } => {
                        if seat.is_some() {
                            let _ = reply.send(Err(spookery_engine::Reason::TableFull.into()));
                        } else {
                            seat = Some(conn);
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
        });
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

    /// Opens the GM's event stream and leaves it open. Dropping the socket
    /// hangs up, like a GM process that dies.
    async fn open_event_stream(port: u16, session: &str, version: &str) -> tokio::net::TcpStream {
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
        let mut head = [0u8; 64];
        let read = stream
            .read(&mut head)
            .await
            .unwrap_or_else(|err| panic!("read: {err}"));
        let head = String::from_utf8_lossy(&head[..read]).into_owned();
        assert!(
            head.contains(" 200 "),
            "the event stream did not open\n{head}"
        );
        stream
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

    async fn bind_door() -> (
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

    async fn sit(port: u16) -> (String, String) {
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

    // A GM that dies without DELETE hangs up its event stream. That frees
    // the seat, so the next GM sits at once (issue #313).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gm_that_hangs_up_frees_the_seat() {
        let (port, mut fault_seen, let_go) = bind_door().await;
        let (session, version) = sit(port).await;
        let events = open_event_stream(port, &session, &version).await;

        drop(events);
        assert!(
            fault_seen.recv().await.is_some(),
            "the seat was never released"
        );
        let_go
            .send(())
            .unwrap_or_else(|err| panic!("let go: {err}"));

        let (again, body) = open(port).await;
        assert!(again.is_some(), "the next GM could not sit\n{body}");
        assert!(
            !body.contains("The table is full"),
            "the next GM could not sit\n{body}"
        );
    }

    // A GM that holds its event stream open but sends nothing keeps the seat
    // past rmcp's old 300 s idle window (issue #313).
    #[tokio::test]
    async fn a_quiet_gm_keeps_the_seat() {
        let (port, mut fault_seen, _let_go) = bind_door().await;
        let (session, version) = sit(port).await;
        let _events = open_event_stream(port, &session, &version).await;

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

    // End to end over HTTP: the handler's drop reaches the table thread, the
    // DELETE reply waits for the release, and a GM that reconnects as soon as
    // DELETE answers is not told the table is full (issue #214).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gm_can_sit_again_as_soon_as_delete_answers() {
        let (port, mut fault_seen, let_go) = bind_door().await;

        let (session, body) = open(port).await;
        let session = session.unwrap_or_else(|| panic!("no session\n{body}"));
        let version = read_negotiated_version(&body);

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
        let (again, body) = open(port).await;
        assert!(again.is_some(), "the GM could not sit again\n{body}");
        assert!(
            !body.contains("The table is full"),
            "the GM could not sit again\n{body}"
        );
    }
}
