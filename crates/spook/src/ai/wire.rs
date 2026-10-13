use serde_json::Value;

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

pub fn status_fields(body: &Value) -> Option<Status> {
    let present = string_list(body.get("presence")?.get("actors_present")?)?;
    let queued = body
        .get("queued")?
        .as_array()?
        .iter()
        .map(|item| {
            Some(crate::ai::prompt::QueueItem {
                id: item.get("id")?.as_str()?.to_string(),
                actor_id: item.get("actor_id")?.as_str()?.to_string(),
                move_name: item.get("move")?.as_str()?.to_string(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let events = match body.get("events") {
        Some(events) => events
            .as_array()?
            .iter()
            .map(say_from_event)
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };
    Some(Status {
        pack_rev: body.get("pack_rev")?.as_str()?.to_string(),
        last_seq: body.get("last_seq")?.as_u64()?,
        present,
        queued,
        events,
    })
}

pub struct Status {
    pub pack_rev: String,
    pub last_seq: u64,
    pub present: Vec<String>,
    pub queued: Vec<crate::ai::prompt::QueueItem>,
    pub events: Vec<crate::ai::prompt::HandledSay>,
}

fn say_from_event(value: &Value) -> Option<crate::ai::prompt::HandledSay> {
    Some(crate::ai::prompt::HandledSay {
        seq: value.get("seq").and_then(Value::as_u64),
        from: value.get("from")?.as_str()?.to_string(),
        address: value.get("address")?.as_str()?.to_string(),
        place: optional_string(value.get("place")),
        actor: optional_string(value.get("actor")),
        text: value.get("text")?.as_str()?.to_string(),
    })
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn string_list(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| Some(item.as_str()?.to_string()))
        .collect()
}

pub fn rolls_of(body: &Value) -> Vec<crate::ai::prompt::RollLine> {
    let Some(checks) = body.get("checks").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in checks {
        let Some(faces) = item.get("faces").and_then(Value::as_array) else {
            continue;
        };
        let faces = faces.iter().filter_map(Value::as_u64).collect::<Vec<_>>();
        let Some(stat) = item.get("stat").and_then(Value::as_i64) else {
            continue;
        };
        let Some(total) = item.get("total").and_then(Value::as_i64) else {
            continue;
        };
        let Some(target) = item.get("target").and_then(Value::as_i64) else {
            continue;
        };
        let Some(outcome) = item.get("outcome").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(outcome, "success" | "partial" | "miss") {
            continue;
        }
        out.push(crate::ai::prompt::RollLine {
            faces,
            stat,
            total,
            target,
            outcome: outcome.to_string(),
        });
    }
    out
}

pub fn say_seq(body: &Value) -> Option<u64> {
    body.get("seq").and_then(Value::as_u64)
}
