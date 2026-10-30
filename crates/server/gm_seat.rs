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
use spookery_engine::Reason;

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
        let (out_tx, out_rx, cancel_tx, cancel_rx) = super::limits::outbound();
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
                // One writer drains this queue, so two tool calls cannot pass each other.
                tokio::spawn(super::limits::write_frames(
                    async move |frame: &[u8]| {
                        emit_event(&peer, frame).await;
                        true
                    },
                    out_rx,
                    cancel_rx,
                ));
                Ok(())
            }
            Err(code) => Err(refusal_error(code)),
        }
    }
}

fn refusal_error(code: Reason) -> McpError {
    McpError::internal_error(crate::protocol::refusal(code.token()), None)
}

async fn emit_event(peer: &Peer<RoleServer>, frame: &[u8]) {
    let Ok(body) = serde_json::from_slice(frame) else {
        return;
    };
    let note = CustomNotification::new("spook/event", Some(body));
    let _ = peer
        .send_notification(ServerNotification::CustomNotification(note))
        .await;
}
