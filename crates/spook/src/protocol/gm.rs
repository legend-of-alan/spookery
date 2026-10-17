use serde::Serialize;
use serde_json::{Map, Value};

use crate::protocol::{Outcome, ProtocolError};

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: u64,
    pub name: String,
    pub arguments: Value,
}

pub fn write_tool_call(call: &ToolCall) -> String {
    let line = serde_json::json!({
        "jsonrpc": "2.0",
        "id": call.id,
        "method": "tools/call",
        "params": {
            "name": call.name,
            "arguments": call.arguments,
        },
    });
    ended(&line)
}

pub fn read_tool_call(line: &str) -> Result<ToolCall, ProtocolError> {
    let value: Value = serde_json::from_str(trim_line(line)).map_err(|_| ProtocolError)?;
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(ProtocolError);
    }
    if value.get("method").and_then(Value::as_str) != Some("tools/call") {
        return Err(ProtocolError);
    }
    let id = value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or(ProtocolError)?;
    let params = value.get("params").ok_or(ProtocolError)?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or(ProtocolError)?
        .to_string();
    let arguments = params.get("arguments").cloned().ok_or(ProtocolError)?;
    Ok(ToolCall {
        id,
        name,
        arguments,
    })
}

/// The structured body of a tool-result line.
pub fn read_tool_result(line: &str) -> Result<Value, ProtocolError> {
    let value: Value = serde_json::from_str(trim_line(line)).map_err(|_| ProtocolError)?;
    let result = value.get("result").ok_or(ProtocolError)?;
    Ok(result
        .get("structuredContent")
        .or_else(|| result.get("structured_content"))
        .cloned()
        .unwrap_or(Value::Null))
}

/// A GM socket line, before the client's turn rules.
pub enum Frame {
    Result {
        id: u64,
    },
    Say {
        from: String,
        seq: u64,
        address: String,
        place: Option<String>,
        actor: Option<String>,
        text: String,
    },
    Queued,
    Dropped,
}

pub fn read_frame(line: &str) -> Result<Frame, ProtocolError> {
    let value: Value = serde_json::from_str(trim_line(line)).map_err(|_| ProtocolError)?;
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        return Ok(Frame::Result { id });
    }
    if value.get("method").and_then(Value::as_str) != Some("spook/event") {
        return Err(ProtocolError);
    }
    let Some(params) = value.get("params") else {
        return Err(ProtocolError);
    };
    let kind = params.get("kind").and_then(Value::as_str).unwrap_or("");
    match kind {
        "check" => Ok(Frame::Dropped),
        "say" => Ok(Frame::Say {
            from: params
                .get("from")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            seq: params.get("seq").and_then(Value::as_u64).unwrap_or(0),
            address: params
                .get("address")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            place: nonempty_string(params.get("place")),
            actor: nonempty_string(params.get("actor")),
            text: params
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        "queued" => Ok(Frame::Queued),
        _ => Ok(Frame::Dropped),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusArgs {
    pub since: Option<u64>,
}

pub fn read_status_args(
    arguments: Option<&Map<String, Value>>,
) -> Result<StatusArgs, ProtocolError> {
    let Some(arguments) = arguments else {
        return Ok(StatusArgs { since: None });
    };
    if arguments.is_empty() {
        return Ok(StatusArgs { since: None });
    }
    if arguments.len() != 1 {
        return Err(ProtocolError);
    }
    let Some(since) = arguments.get("since").and_then(Value::as_u64) else {
        return Err(ProtocolError);
    };
    Ok(StatusArgs { since: Some(since) })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerceiveArgs {
    pub as_viewer: Option<String>,
    pub focus: Option<String>,
    pub budget_tokens: Option<u64>,
}

pub fn read_perceive_args(
    arguments: Option<&Map<String, Value>>,
) -> Result<PerceiveArgs, ProtocolError> {
    let empty = Map::new();
    let arguments = arguments.unwrap_or(&empty);
    let mut as_viewer = None;
    let mut focus = None;
    let mut budget_tokens = None;
    for (key, value) in arguments {
        match key.as_str() {
            "as" => as_viewer = Some(value.as_str().ok_or(ProtocolError)?.to_string()),
            "focus" => match value.as_str() {
                Some(text @ ("place" | "actor")) => focus = Some(text.to_string()),
                _ => return Err(ProtocolError),
            },
            "budget_tokens" => budget_tokens = Some(value.as_u64().ok_or(ProtocolError)?),
            _ => return Err(ProtocolError),
        }
    }
    Ok(PerceiveArgs {
        as_viewer,
        focus,
        budget_tokens,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitArgs {
    pub include_tick: bool,
    pub session_id: Option<String>,
}

pub fn read_commit_args(
    arguments: Option<&Map<String, Value>>,
) -> Result<CommitArgs, ProtocolError> {
    let arguments = arguments.ok_or(ProtocolError)?;
    let mut include_tick = None;
    let mut session_id = None;
    for (key, value) in arguments {
        match key.as_str() {
            "include_tick" => include_tick = Some(value.as_bool().ok_or(ProtocolError)?),
            "session_id" => session_id = Some(value.as_str().ok_or(ProtocolError)?.to_string()),
            _ => return Err(ProtocolError),
        }
    }
    Ok(CommitArgs {
        include_tick: include_tick.ok_or(ProtocolError)?,
        session_id,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SayArgs {
    pub address: String,
    pub text: String,
    pub place: Option<String>,
    pub actor: Option<String>,
}

pub fn read_say_args(arguments: Option<&Map<String, Value>>) -> Result<SayArgs, ProtocolError> {
    let arguments = arguments.ok_or(ProtocolError)?;
    let mut address = None;
    let mut place = None;
    let mut actor = None;
    let mut text = None;
    for (key, value) in arguments {
        match key.as_str() {
            "from" => return Err(ProtocolError),
            "address" => address = Some(value.as_str().ok_or(ProtocolError)?.to_string()),
            "place" => place = optional_text(value)?,
            "actor" => actor = optional_text(value)?,
            "text" => text = Some(value.as_str().ok_or(ProtocolError)?.to_string()),
            _ => return Err(ProtocolError),
        }
    }
    Ok(SayArgs {
        address: address.ok_or(ProtocolError)?,
        text: text.ok_or(ProtocolError)?,
        place,
        actor,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallTarget {
    Empty,
    Place(String),
    Actor(String),
    Clock(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentCall {
    pub id: Option<String>,
    pub tick_index: Option<u64>,
    pub actor_id: Option<String>,
    pub actor_kind: Option<String>,
    pub source: Option<String>,
    pub move_name: Option<String>,
    pub target: CallTarget,
    pub message_to: Option<Vec<String>>,
    pub rationale_code: String,
}

pub fn read_intent_args(
    arguments: Option<&Map<String, Value>>,
) -> Result<IntentCall, ProtocolError> {
    let arguments = arguments.ok_or(ProtocolError)?;
    let mut id = None;
    let mut tick_index = None;
    let mut actor_id = None;
    let mut actor_kind = None;
    let mut source = None;
    let mut move_name = None;
    let mut target = CallTarget::Empty;
    let mut message_to = None;
    let mut rationale_code = String::new();
    for (key, value) in arguments {
        match key.as_str() {
            "id" => id = Some(value.as_str().ok_or(ProtocolError)?.to_string()),
            "tick_index" => tick_index = Some(value.as_u64().ok_or(ProtocolError)?),
            "actor_id" => actor_id = Some(value.as_str().ok_or(ProtocolError)?.to_string()),
            "actor_kind" => actor_kind = Some(value.as_str().ok_or(ProtocolError)?.to_string()),
            "source" => source = Some(value.as_str().ok_or(ProtocolError)?.to_string()),
            "move" => move_name = Some(value.as_str().ok_or(ProtocolError)?.to_string()),
            "target" => target = read_target(value)?,
            "args" => message_to = read_message_to(value)?,
            "rationale_code" => rationale_code = value.as_str().ok_or(ProtocolError)?.to_string(),
            _ => return Err(ProtocolError),
        }
    }
    Ok(IntentCall {
        id,
        tick_index,
        actor_id,
        actor_kind,
        source,
        move_name,
        target,
        message_to,
        rationale_code,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionStatus {
    pub presence: StatusPresence,
    pub pause: StatusPause,
    pub world_time: u64,
    pub tick_index: u64,
    pub state_rev: u64,
    pub pack_rev: String,
    pub last_tick_at: String,
    pub queued: Vec<QueueRow>,
    pub last_seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<Value>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatusPresence {
    pub humans: u32,
    pub actors_present: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatusPause {
    pub named_plot_clocks: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct QueueRow {
    pub id: String,
    pub actor_id: String,
    #[serde(rename = "move")]
    pub move_name: String,
}

impl SessionStatus {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("session status")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommitBody {
    pub state_rev: u64,
    pub diff_summary: Vec<String>,
    pub log_path: Option<String>,
    pub checks: Vec<Roll>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dice: Vec<String>,
}

impl CommitBody {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("commit")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Roll {
    pub faces: Vec<u32>,
    pub stat: i64,
    pub total: i64,
    pub target: i64,
    pub outcome: Outcome,
}

pub fn set_checks(value: &mut Value, rolls: &[Roll]) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert(
        "checks".to_string(),
        serde_json::to_value(rolls).expect("checks"),
    );
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GmSay {
    pub seq: u64,
    pub from: String,
    pub address: String,
    pub place: Option<String>,
    pub actor: Option<String>,
    pub recipients: Vec<String>,
    pub text: String,
}

impl GmSay {
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "kind": "say",
            "seq": self.seq,
            "from": self.from,
            "address": self.address,
            "place": self.place,
            "actor": self.actor,
            "recipients": self.recipients,
            "text": self.text,
        })
    }
}

pub fn write_say_event(say: &GmSay) -> Vec<u8> {
    event_line(say.to_value())
}

pub fn write_queued_event(id: &str, actor_id: &str, move_name: &str) -> Vec<u8> {
    event_line(serde_json::json!({
        "kind": "queued",
        "id": id,
        "actor_id": actor_id,
        "move": move_name,
    }))
}

pub fn write_check_event(roll: &Roll) -> Vec<u8> {
    event_line(serde_json::json!({
        "kind": "check",
        "faces": roll.faces,
        "stat": roll.stat,
        "total": roll.total,
        "target": roll.target,
        "outcome": roll.outcome.as_str(),
    }))
}

pub fn queued_ack(id: &str) -> Value {
    serde_json::json!({ "queued": id })
}

pub fn code_result(code: &str) -> Value {
    serde_json::json!({ "code": code })
}

fn read_target(value: &Value) -> Result<CallTarget, ProtocolError> {
    let object = value.as_object().ok_or(ProtocolError)?;
    if object.is_empty() {
        return Ok(CallTarget::Empty);
    }
    if object.len() != 1 {
        return Err(ProtocolError);
    }
    if let Some(place) = object.get("place").and_then(Value::as_str) {
        return Ok(CallTarget::Place(place.to_string()));
    }
    if let Some(actor) = object.get("actor").and_then(Value::as_str) {
        return Ok(CallTarget::Actor(actor.to_string()));
    }
    if let Some(clock) = object.get("clock").and_then(Value::as_str) {
        return Ok(CallTarget::Clock(clock.to_string()));
    }
    Err(ProtocolError)
}

fn read_message_to(value: &Value) -> Result<Option<Vec<String>>, ProtocolError> {
    let object = value.as_object().ok_or(ProtocolError)?;
    let mut message_to = None;
    for (key, item) in object {
        match key.as_str() {
            "to" => {
                let values = item.as_array().ok_or(ProtocolError)?;
                let mut ids = Vec::new();
                for value in values {
                    ids.push(value.as_str().ok_or(ProtocolError)?.to_string());
                }
                message_to = Some(ids);
            }
            _ => return Err(ProtocolError),
        }
    }
    Ok(message_to)
}

fn optional_text(value: &Value) -> Result<Option<String>, ProtocolError> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => Err(ProtocolError),
    }
}

fn nonempty_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn event_line(params: Value) -> Vec<u8> {
    let line = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "spook/event",
        "params": params,
    });
    ended(&line).into_bytes()
}

fn ended(value: &Value) -> String {
    let mut line = serde_json::to_string(value).expect("gm line");
    line.push('\n');
    line
}

fn trim_line(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_call_round_trips() {
        let call = ToolCall {
            id: 2,
            name: "say".to_string(),
            arguments: serde_json::json!({
                "address": "table",
                "text": "evening"
            }),
        };
        let line = write_tool_call(&call);
        assert_eq!(
            line,
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"say\",\"arguments\":{\"address\":\"table\",\"text\":\"evening\"}}}\n"
        );
        assert_eq!(read_tool_call(&line).unwrap(), call);
    }

    #[test]
    fn a_say_event_keeps_null_place_and_actor() {
        let say = GmSay {
            seq: 3,
            from: "gm".to_string(),
            address: "table".to_string(),
            place: None,
            actor: None,
            recipients: Vec::new(),
            text: "evening".to_string(),
        };
        let line = String::from_utf8(write_say_event(&say)).unwrap();
        assert_eq!(
            line,
            "{\"jsonrpc\":\"2.0\",\"method\":\"spook/event\",\"params\":{\"kind\":\"say\",\"seq\":3,\"from\":\"gm\",\"address\":\"table\",\"place\":null,\"actor\":null,\"recipients\":[],\"text\":\"evening\"}}\n"
        );
    }
}
