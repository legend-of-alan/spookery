//! Turn a model line into a tool result.

use crate::ai::fail;
use crate::ai::HostError;
use crate::gm_connection::ToolCallResult;

pub fn parse_tool_line(raw: &str) -> Result<ToolCallResult, HostError> {
    let body = crate::protocol::read_tool_result(raw).map_err(|_| fail(raw.to_string()))?;
    let refused = body.get("code").is_some();
    Ok(ToolCallResult {
        raw: raw.to_string(),
        body,
        refused,
    })
}
