use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, CustomNotification,
    InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
    ServerConfig, ServerNotification, Tool, ToolsCapability,
};
use rmcp::service::{Peer, RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};

use super::listen;
use super::thread::{HelloReply, Msg, SocketEnd, ToolReply};
use super::tools;

struct HttpSeat {
    conn: u64,
    held: Arc<AtomicBool>,
    peer: Arc<tokio::sync::Mutex<Option<Peer<RoleServer>>>>,
}

pub(super) struct GmHost {
    tx: Sender<Msg>,
    after_tx: std::sync::mpsc::Sender<Vec<u8>>,
    http: Option<HttpSeat>,
}

pub(super) fn http_host(tx: Sender<Msg>, conn: u64) -> GmHost {
    let (after_tx, after_rx) = std::sync::mpsc::channel();
    drop(after_rx);
    GmHost {
        tx,
        after_tx,
        http: Some(HttpSeat {
            conn,
            held: Arc::new(AtomicBool::new(false)),
            peer: Arc::new(tokio::sync::Mutex::new(None)),
        }),
    }
}

pub(super) async fn serve(
    read_half: tokio::net::tcp::OwnedReadHalf,
    write_half: tokio::net::tcp::OwnedWriteHalf,
    out_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    out_rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    tx: Sender<Msg>,
    conn: u64,
) {
    let (after_tx, after_rx) = std::sync::mpsc::channel();
    let writer_task = tokio::spawn(async move {
        let mut out_rx = out_rx;
        let mut write_half = write_half;
        while let Some(frame) = out_rx.recv().await {
            if write_frame(&mut write_half, &frame).await.is_err() {
                break;
            }
        }
    });
    let writer = LineWriter {
        tx: out_tx,
        buf: Vec::new(),
        after_rx,
    };
    let host = GmHost {
        tx: tx.clone(),
        after_tx,
        http: None,
    };
    if let Ok(running) = host.serve((read_half, writer)).await {
        let _ = running.waiting().await;
    }
    listen::report_fault(&tx, conn, SocketEnd::Eof).await;
    let _ = writer_task.await;
}

async fn write_frame(write: &mut tokio::net::tcp::OwnedWriteHalf, frame: &[u8]) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    write.write_all(frame).await?;
    write.flush().await?;
    Ok(())
}

struct LineWriter {
    tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    buf: Vec<u8>,
    after_rx: std::sync::mpsc::Receiver<Vec<u8>>,
}

impl tokio::io::AsyncWrite for LineWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let writer = self.get_mut();
        writer.buf.extend_from_slice(data);
        while let Some(index) = writer.buf.iter().position(|byte| *byte == b'\n') {
            let frame: Vec<u8> = writer.buf.drain(..=index).collect();
            if writer.tx.send(frame).is_err() {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "gm writer closed",
                )));
            }
            while let Ok(extra) = writer.after_rx.try_recv() {
                if writer.tx.send(extra).is_err() {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "gm writer closed",
                    )));
                }
            }
        }
        Poll::Ready(Ok(data.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl Drop for GmHost {
    fn drop(&mut self) {
        self.release();
    }
}

impl ServerHandler for GmHost {
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
        let tools = tools::GM_LIST
            .iter()
            .map(|tool| {
                Tool::new(
                    tool.name,
                    tool.description(),
                    std::sync::Arc::new((tool.schema)()),
                )
            })
            .collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        if self.http.is_some() {
            self.claim(&context).await?;
        }
        context.peer.set_peer_info(request.clone());
        match self.negotiate_initialize(&request) {
            Ok(info) => Ok(info),
            Err(err) => {
                self.release();
                Err(err)
            }
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let tx = self.tx.clone();
        let after_tx = self.after_tx.clone();
        let peer = self.http.as_ref().map(|http| Arc::clone(&http.peer));
        let name = request.name.into_owned();
        let arguments = request.arguments;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(Msg::Tool {
            name,
            arguments,
            reply: reply_tx,
        })
        .map_err(|_| McpError::internal_error("shard closed", None))?;
        let ToolReply { value, after } = reply_rx
            .await
            .map_err(|_| McpError::internal_error("shard closed", None))?;
        for frame in after {
            let _ = after_tx.send(frame.clone());
            if let Some(peer) = &peer {
                let guard = peer.lock().await;
                if let Some(peer) = guard.as_ref() {
                    emit_event(peer, &frame).await;
                }
            }
        }
        Ok(CallToolResult::structured(value).into())
    }
}

impl GmHost {
    async fn claim(&self, context: &RequestContext<RoleServer>) -> Result<(), McpError> {
        let Some(http) = &self.http else {
            return Ok(());
        };
        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Msg::Hello {
                conn: http.conn,
                line: "gm".to_string(),
                out: out_tx,
                reply: reply_tx,
            })
            .map_err(|_| McpError::internal_error("shard closed", None))?;
        let reply = reply_rx
            .await
            .map_err(|_| McpError::internal_error("shard closed", None))?;
        match reply {
            HelloReply::Gm => {
                http.held.store(true, Ordering::Release);
                {
                    let mut slot = http.peer.lock().await;
                    *slot = Some(context.peer.clone());
                }
                let peer = context.peer.clone();
                tokio::spawn(async move {
                    while let Some(frame) = out_rx.recv().await {
                        emit_event(&peer, &frame).await;
                    }
                });
                Ok(())
            }
            HelloReply::Refused(line) => Err(refusal_error(&line)),
            HelloReply::Player => Err(McpError::internal_error("shard closed", None)),
        }
    }

    fn release(&self) {
        let Some(http) = &self.http else {
            return;
        };
        if !http.held.swap(false, Ordering::AcqRel) {
            return;
        }
        let (ack_tx, mut ack_rx) = tokio::sync::oneshot::channel();
        if self
            .tx
            .send(Msg::Fault {
                conn: http.conn,
                end: SocketEnd::Close,
                ack: ack_tx,
            })
            .is_err()
        {
            return;
        }
        let start = Instant::now();
        loop {
            match ack_rx.try_recv() {
                Ok(()) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => return,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    if start.elapsed() > Duration::from_secs(2) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

fn refusal_error(line: &str) -> McpError {
    let code = serde_json::from_str::<serde_json::Value>(line.trim())
        .ok()
        .and_then(|value| {
            value
                .get("code")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "SESSION_FULL".to_string());
    let sentence = if code == "SESSION_FULL" {
        format!("The table is full. [{code}]")
    } else {
        format!("The table refused that. [{code}]")
    };
    McpError::internal_error(sentence, None)
}

async fn emit_event(peer: &Peer<RoleServer>, frame: &[u8]) {
    let Ok(text) = std::str::from_utf8(frame) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return;
    };
    let Some(method) = value.get("method").and_then(serde_json::Value::as_str) else {
        return;
    };
    let params = value.get("params").cloned();
    let note = ServerNotification::CustomNotification(CustomNotification::new(method, params));
    let _ = peer.send_notification(note).await;
}
