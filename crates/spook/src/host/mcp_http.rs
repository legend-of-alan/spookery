//! Serve the agent door over HTTP.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use futures_core::Stream;
use rmcp::model::{ClientJsonRpcMessage, GetExtensions, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_server::session::local::{
    LocalSessionManager, LocalSessionManagerError,
};
use rmcp::transport::streamable_http_server::session::{
    ServerSseMessage, SessionId, SessionManager,
};
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::sync::oneshot;

use super::gm_seat;
use super::limits::GM_BODY_CAP;
use super::world_loop::Msg;

// HTTP POST bodies on the agent door stop at this cap.
pub(super) async fn serve(
    listener: tokio::net::TcpListener,
    tx: Sender<Msg>,
    next_conn: Arc<AtomicU64>,
) {
    let service = StreamableHttpService::new(
        move || {
            let conn = next_conn.fetch_add(1, Ordering::Relaxed);
            Ok(gm_seat::http_host(tx.clone(), conn))
        },
        Arc::new(SeatSessions::default()),
        http_config(),
    );
    let app = axum::Router::new().route_service("/mcp", service);
    let _ = axum::serve(listener, app).await;
}

fn http_config() -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["127.0.0.1"])
        .with_allowed_origins(["http://127.0.0.1:*"]);
    config.max_request_body_bytes = GM_BODY_CAP;
    config
}

/// Rides on a session's `initialize` request to its `GmHost`. The host takes
/// the sender and drops it after its seat is free, so the receiver resolves
/// once the shard thread no longer holds the GM seat for that session.
#[derive(Clone)]
pub(super) struct SeatReleased(Arc<Mutex<Option<oneshot::Sender<()>>>>);

impl SeatReleased {
    pub(super) fn take(&self) -> Option<oneshot::Sender<()>> {
        self.0.lock().unwrap_or_else(|err| err.into_inner()).take()
    }
}

/// rmcp's local sessions, except that closing a session (HTTP DELETE) answers
/// only after that session's GM seat is released. rmcp's own close only queues
/// the close for the session worker, and the handler, which holds the seat,
/// drops later on another task. A GM that reconnects as soon as DELETE returns
/// would otherwise find its old seat still taken.
#[derive(Default)]
struct SeatSessions {
    inner: LocalSessionManager,
    released: Mutex<HashMap<SessionId, oneshot::Receiver<()>>>,
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
            let (tx, rx) = oneshot::channel();
            self.released
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .insert(id.clone(), rx);
            request
                .request
                .extensions_mut()
                .insert(SeatReleased(Arc::new(Mutex::new(Some(tx)))));
        }
        self.inner.initialize_session(id, message).await
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        self.inner.has_session(id).await
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        let released = self
            .released
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(id);
        self.inner.close_session(id).await?;
        // The sender drops with the handler, after its seat is free, so this
        // ends on every path: released, refused, or never claimed.
        if let Some(released) = released {
            let _ = released.await;
        }
        Ok(())
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
        self.inner.create_standalone_stream(id).await
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        self.inner.resume(id, last_event_id).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
    use std::sync::{mpsc, Arc};

    use rmcp::model::ProtocolVersion;
    use serde_json::Value;

    use super::super::world_loop::{Ack, Msg};
    use super::{serve, SeatSessions, SessionId, SessionManager};

    // Close only answers once the session's seat release is confirmed. The
    // release here is a bare sender, so nothing else can wake the close.
    #[tokio::test]
    async fn closing_a_session_waits_for_its_seat_release() {
        use std::future::Future;
        use std::task::{Context, Poll, Waker};

        let sessions = SeatSessions::default();
        let id: SessionId = Arc::from("held");
        let (released, confirm) = tokio::sync::oneshot::channel::<()>();
        sessions
            .released
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(id.clone(), confirm);
        let mut close = std::pin::pin!(sessions.close_session(&id));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(
            close.as_mut().poll(&mut cx).is_pending(),
            "close answered while the seat was still held"
        );
        drop(released);
        assert!(
            matches!(close.as_mut().poll(&mut cx), Poll::Ready(Ok(()))),
            "close did not answer once the seat was free"
        );
    }

    /// A stand-in for the shard thread with one GM seat. It refuses a second
    /// claim while the seat is held, like the real table. On a fault it tells
    /// the test, then holds the seat until the test lets the release through.
    fn fake_world_loop(
        rx: mpsc::Receiver<Msg>,
        fault_seen: tokio::sync::mpsc::UnboundedSender<()>,
        let_go: mpsc::Receiver<()>,
    ) {
        std::thread::spawn(move || {
            let mut seat: Option<u64> = None;
            let mut kept = Vec::new();
            for msg in rx {
                match msg {
                    Msg::Claim {
                        conn,
                        out,
                        cancel,
                        reply,
                    } => {
                        if seat.is_some() {
                            let _ = reply.send(Err("{\"code\":\"SESSION_FULL\"}".to_string()));
                        } else {
                            seat = Some(conn);
                            kept.push((out, cancel));
                            let _ = reply.send(Ok(()));
                        }
                    }
                    Msg::Fault { conn, ack, .. } => {
                        let _ = fault_seen.send(());
                        if let_go.recv().is_err() {
                            return;
                        }
                        if seat == Some(conn) {
                            seat = None;
                        }
                        match ack {
                            Ack::Thread(tx) => {
                                let _ = tx.try_send(());
                            }
                            Ack::Task(tx) => {
                                let _ = tx.send(());
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    fn initialize() -> Value {
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
    async fn http(
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
        let raw = http(
            port,
            "POST",
            None,
            ProtocolVersion::LATEST.as_str(),
            &initialize().to_string(),
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

    fn negotiated(body: &str) -> String {
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

    // End to end over HTTP: the handler's drop reaches the shard thread, the
    // DELETE reply waits for the release, and a GM that reconnects as soon as
    // DELETE answers is not told the table is full (issue #214).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gm_can_sit_again_as_soon_as_delete_answers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|err| panic!("bind: {err}"));
        let port = listener
            .local_addr()
            .unwrap_or_else(|err| panic!("addr: {err}"))
            .port();
        let (tx, rx) = mpsc::channel();
        let (fault_tx, mut fault_seen) = tokio::sync::mpsc::unbounded_channel();
        let (let_go, held) = mpsc::channel();
        fake_world_loop(rx, fault_tx, held);
        tokio::spawn(serve(listener, tx, Arc::new(AtomicU64::new(1))));

        let (session, body) = open(port).await;
        let session = session.unwrap_or_else(|| panic!("no session\n{body}"));
        let version = negotiated(&body);

        let mut delete =
            tokio::spawn(async move { http(port, "DELETE", Some(&session), &version, "").await });

        // The release reaches the shard thread, which holds the seat for now.
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
            "DELETE answered while the shard thread still held the seat"
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
