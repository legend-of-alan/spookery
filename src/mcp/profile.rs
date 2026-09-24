use std::path::Path;
use std::sync::Arc;

use rmcp::model::{JsonObject, ListToolsResult, PaginatedRequestParam, ServerInfo, Tool, ToolsCapability};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use spookery_kernel::Code;

const TABLE_TOOLS: &[&str] = &[
    "session_open",
    "session_status",
    "perceive",
    "intent",
    "resolve_check",
    "apply_effect",
    "npc_act",
    "commit_turn",
    "recall",
    "session_save",
];

pub fn serve_table(pack: &Path) -> Result<(), Code> {
    let _shard = spookery_kernel::open_shard(pack)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Code::NotInSpec)?;
    runtime.block_on(async {
        let running = TableServer
            .serve((tokio::io::stdin(), tokio::io::stdout()))
            .await
            .map_err(|_| Code::NotInSpec)?;
        running.waiting().await.map_err(|_| Code::NotInSpec)?;
        Ok(())
    })
}

struct TableServer;

impl ServerHandler for TableServer {
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
        let schema = Arc::new(object_schema());
        let tools = TABLE_TOOLS
            .iter()
            .map(|name| Tool::new(*name, *name, Arc::clone(&schema)))
            .collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }
}

fn object_schema() -> JsonObject {
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), "object".into());
    schema
}
