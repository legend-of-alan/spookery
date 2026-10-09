use std::io;
use std::pin::Pin;
use std::sync::mpsc::Sender;
use std::task::{Context, Poll};

use rmcp::model::{
    CallToolRequestParam, CallToolResult, ListToolsResult, PaginatedRequestParam, ServerInfo, Tool,
    ToolsCapability,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};

use super::listen;
use super::thread::{Msg, SocketEnd, ToolReply};
use super::tools;

#[derive(Clone)]
struct GmHost {
    tx: Sender<Msg>,
    after_tx: std::sync::mpsc::Sender<Vec<u8>>,
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

impl ServerHandler for GmHost {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities.tools = Some(ToolsCapability { list_changed: None });
        info
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
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

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let tx = self.tx.clone();
        let after_tx = self.after_tx.clone();
        let name = request.name.into_owned();
        let arguments = request.arguments;
        async move {
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
                let _ = after_tx.send(frame);
            }
            Ok(CallToolResult::structured(value))
        }
    }
}
