//! The MCP server that lists and runs the GM tools.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, CustomNotification,
    InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
    ServerConfig, ServerNotification, Tool, ToolsCapability,
};
use rmcp::service::{Peer, RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler};
use spookery_kernel::api::Reason;

use super::gm_tools;
use super::world_loop::{Ack, Msg, SocketEnd, ToolReply};

/// How long releasing an HTTP GM seat waits for the shard thread's ack.
const RELEASE_WAIT: Duration = Duration::from_secs(2);

struct HttpSeat {
    conn: u64,
    held: Arc<AtomicBool>,
    /// Clone of the seat's outbound queue. Say and check frames go here, in
    /// order, and the one forwarder below writes them.
    out: std::sync::Mutex<Option<super::limits::OutTx>>,
    /// Dropped after the seat is free. A DELETE on this session waits for it.
    released: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

pub(super) struct GmHost {
    tx: Sender<Msg>,
    http: Option<HttpSeat>,
}

pub(super) fn http_host(tx: Sender<Msg>, conn: u64) -> GmHost {
    GmHost {
        tx,
        http: Some(HttpSeat {
            conn,
            held: Arc::new(AtomicBool::new(false)),
            out: std::sync::Mutex::new(None),
            released: std::sync::Mutex::new(None),
        }),
    }
}

impl Drop for GmHost {
    fn drop(&mut self) {
        self.release();
        // The seat is free, so the session's DELETE may answer now.
        if let Some(http) = &self.http {
            drop(
                http.released
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .take(),
            );
        }
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
        let tools = gm_tools::GM_LIST
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
        if let Some(http) = &self.http {
            if let Some(tag) = context.extensions.get::<super::mcp_http::SeatRelease>() {
                *http.released.lock().unwrap_or_else(|err| err.into_inner()) = tag.take();
            }
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
        let conn = {
            let Some(http) = &self.http else {
                return Err(McpError::internal_error("no seat", None));
            };
            if !http.held.load(Ordering::Acquire) {
                return Ok(CallToolResult::structured(crate::protocol::code_result(
                    Reason::NotInSpec.token(),
                ))
                .into());
            }
            http.conn
        };
        let tx = self.tx.clone();
        let name = request.name.into_owned();
        let arguments = request.arguments;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(Msg::Tool {
            conn,
            name,
            arguments,
            reply: reply_tx,
        })
        .map_err(|_| McpError::internal_error("shard closed", None))?;
        let ToolReply {
            value,
            after,
            released,
        } = reply_rx
            .await
            .map_err(|_| McpError::internal_error("shard closed", None))?;
        if released {
            if let Some(http) = &self.http {
                http.held.store(false, Ordering::Release);
            }
        }
        // Queue the events and return. Waiting here for the write stalls the
        // client: it is blocked on this result, so it never reads the event.
        // The forwarder started in `claim` drains this queue in order.
        self.enqueue_after(after);
        Ok(CallToolResult::structured(value).into())
    }
}

impl GmHost {
    async fn claim(&self, context: &RequestContext<RoleServer>) -> Result<(), McpError> {
        let Some(http) = &self.http else {
            return Ok(());
        };
        let (out_tx, mut out_rx, cancel_tx, mut cancel_rx) = super::limits::outbound();
        let queued = out_tx.clone();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Msg::Claim {
                conn: http.conn,
                out: out_tx,
                cancel: cancel_tx,
                reply: reply_tx,
            })
            .map_err(|_| McpError::internal_error("shard closed", None))?;
        let reply = reply_rx
            .await
            .map_err(|_| McpError::internal_error("shard closed", None))?;
        match reply {
            Ok(()) => {
                http.held.store(true, Ordering::Release);
                *http.out.lock().unwrap_or_else(|err| err.into_inner()) = Some(queued);
                let peer = context.peer.clone();
                tokio::spawn(async move {
                    loop {
                        let item = tokio::select! {
                            biased;
                            item = out_rx.recv() => item,
                            changed = cancel_rx.changed() => {
                                if changed.is_err() || *cancel_rx.borrow() {
                                    break;
                                }
                                continue;
                            }
                        };
                        let Some(item) = item else {
                            break;
                        };
                        let (frame, ack) = match item {
                            super::limits::Outbound::Frame(frame) => (frame, None),
                            super::limits::Outbound::Last(frame, ack) => (frame, Some(ack)),
                        };
                        let send = emit_event(&peer, &frame);
                        tokio::pin!(send);
                        let stop = tokio::select! {
                            _ = &mut send => {
                                out_rx.release_written(&frame);
                                false
                            }
                            changed = cancel_rx.changed() => {
                                changed.is_err() || *cancel_rx.borrow()
                            }
                        };
                        if let Some(ack) = ack {
                            let _ = ack.send(());
                        }
                        if stop {
                            break;
                        }
                    }
                });
                Ok(())
            }
            Err(line) => Err(refusal_error(&line)),
        }
    }

    /// One forwarder writes this queue, so two tool calls cannot pass each
    /// other. A full queue drops the seat.
    fn enqueue_after(&self, frames: Vec<Vec<u8>>) {
        let Some(http) = &self.http else {
            return;
        };
        // Hold the queue lock across every frame of this call, then drop it
        // before a full queue releases the seat.
        let full = {
            let guard = http.out.lock().unwrap_or_else(|err| err.into_inner());
            let Some(out) = guard.as_ref() else {
                return;
            };
            let mut full = false;
            for frame in frames {
                if !super::limits::try_frame(out, frame) {
                    full = true;
                    break;
                }
            }
            full
        };
        if full {
            self.release();
        }
    }

    fn release(&self) {
        let Some(http) = &self.http else {
            return;
        };
        if !http.held.swap(false, Ordering::AcqRel) {
            return;
        }
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
        if self
            .tx
            .send(Msg::Fault {
                conn: http.conn,
                end: SocketEnd::Close,
                ack: Ack::Thread(ack_tx),
            })
            .is_err()
        {
            return;
        }
        // Wait until the shard thread has freed the seat, for at most RELEASE_WAIT.
        // A dropped sender (the shard thread is gone) ends the wait at once.
        // This runs from Drop, often on a tokio worker, so on the multi-thread
        // runtime the wait is moved off the worker with block_in_place.
        let wait = move || {
            let _ = ack_rx.recv_timeout(RELEASE_WAIT);
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(wait);
            }
            _ => wait(),
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
    let sentence = match code.as_str() {
        "SESSION_FULL" => format!("The table is full. [{code}]"),
        "LINE_TOO_LONG" => format!("That line is too long. [{code}]"),
        _ => format!("The table refused that. [{code}]"),
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
