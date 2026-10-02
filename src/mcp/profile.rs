use std::path::Path;
use std::sync::Arc;

use rmcp::model::{
    JsonObject, ListToolsResult, PaginatedRequestParam, ServerInfo, Tool, ToolsCapability,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use spookery_kernel::Reason;

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

const TICK_TOOLS: &[&str] = &[
    "session_status",
    "perceive",
    "recall",
    "npc_act",
    "resolve_check",
    "apply_effect",
    "tick_commit",
];

const STUDIO_TOOLS: &[&str] = &[
    "pack_info",
    "read_entity",
    "patch_entity",
    "lint_pack",
    "simulate_knowledge",
    "run_conformance",
    "compile_preview",
];

#[derive(Clone, Copy)]
pub enum Profile {
    Table,
    Tick,
    Studio,
}

pub fn parse_profile(name: &str) -> Option<Profile> {
    match name {
        "pack-table" => Some(Profile::Table),
        "pack-tick" => Some(Profile::Tick),
        "pack-studio" => Some(Profile::Studio),
        _ => None,
    }
}

fn tool_names(profile: Profile) -> &'static [&'static str] {
    match profile {
        Profile::Table => TABLE_TOOLS,
        Profile::Tick => TICK_TOOLS,
        Profile::Studio => STUDIO_TOOLS,
    }
}

pub fn serve(pack: &Path, profile: Profile) -> Result<(), Reason> {
    let _shard = spookery_kernel::open_shard(pack)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Reason::NotInSpec)?;
    runtime.block_on(async {
        let running = ToolServer { profile }
            .serve((tokio::io::stdin(), tokio::io::stdout()))
            .await
            .map_err(|_| Reason::NotInSpec)?;
        running.waiting().await.map_err(|_| Reason::NotInSpec)?;
        Ok(())
    })
}

struct ToolServer {
    profile: Profile,
}

impl ServerHandler for ToolServer {
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
        let tools = tool_names(self.profile)
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
