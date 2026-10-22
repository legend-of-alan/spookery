//! The MCP server that lists and runs the GM tools.

use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, CustomNotification,
    InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
    ServerConfig, ServerNotification, Tool, ToolsCapability,
};
use rmcp::service::{Peer, RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler};

use super::gm_tools;
use super::world_loop::WorldMsg;

pub(super) struct GmSeatHandler {
    tx: Sender<WorldMsg>,
    conn: OnceLock<u64>,
}

impl GmSeatHandler {
    pub(super) fn new(tx: Sender<WorldMsg>) -> Self {
        Self {
            tx,
            conn: OnceLock::new(),
        }
    }
}

impl ServerHandler for GmSeatHandler {
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
        let conn = context
            .extensions
            .get::<super::mcp_http::SeatConn>()
            .map(|seat| seat.0)
            .ok_or_else(|| McpError::internal_error("no seat", None))?;
        let _ = self.conn.set(conn);
        self.claim(conn, &context).await?;
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
        tx.send(WorldMsg::Tool {
            conn,
            name,
            arguments,
            reply: reply_tx,
        })
        .map_err(|_| McpError::internal_error("shard closed", None))?;
        let value = reply_rx
            .await
            .map_err(|_| McpError::internal_error("shard closed", None))?;
        Ok(CallToolResult::structured(value).into())
    }
}

impl GmSeatHandler {
    async fn claim(&self, conn: u64, context: &RequestContext<RoleServer>) -> Result<(), McpError> {
        let (out_tx, mut out_rx, cancel_tx, mut cancel_rx) = super::limits::outbound();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(WorldMsg::Claim {
                conn,
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
                let peer = context.peer.clone();
                // One forwarder writes this queue, so two tool calls cannot pass each other.
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
