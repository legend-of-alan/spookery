//! JSON lines for the GM seat: tool calls, events, and status.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::protocol::{Error, Outcome};

/// The structured body of a tool-result line.
pub fn read_tool_result(line: &str) -> Result<Value, Error> {
    let value: Value = serde_json::from_str(trim_line(line)).map_err(|_| Error)?;
    let result = value.get("result").ok_or(Error)?;
    Ok(result
        .get("structuredContent")
        .or_else(|| result.get("structured_content"))
        .cloned()
        .unwrap_or(Value::Null))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusArgs {
    pub since: Option<u64>,
}

pub fn read_status_args(arguments: Option<&Map<String, Value>>) -> Result<StatusArgs, Error> {
    let Some(arguments) = arguments else {
        return Ok(StatusArgs { since: None });
    };
    if arguments.is_empty() {
        return Ok(StatusArgs { since: None });
    }
    if arguments.len() != 1 {
        return Err(Error);
    }
    let Some(since) = arguments.get("since").and_then(Value::as_u64) else {
        return Err(Error);
    };
    Ok(StatusArgs { since: Some(since) })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerceiveArgs {
    pub as_viewer: Option<String>,
    pub focus: Option<String>,
    pub budget_tokens: Option<u64>,
}

pub fn read_perceive_args(arguments: Option<&Map<String, Value>>) -> Result<PerceiveArgs, Error> {
    let empty = Map::new();
    let arguments = arguments.unwrap_or(&empty);
    let mut as_viewer = None;
    let mut focus = None;
    let mut budget_tokens = None;
    for (key, value) in arguments {
        match key.as_str() {
            "as" => as_viewer = Some(value.as_str().ok_or(Error)?.to_string()),
            "focus" => match value.as_str() {
                Some(text @ ("place" | "actor")) => focus = Some(text.to_string()),
                _ => return Err(Error),
            },
            "budget_tokens" => budget_tokens = Some(value.as_u64().ok_or(Error)?),
            _ => return Err(Error),
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

pub fn read_commit_args(arguments: Option<&Map<String, Value>>) -> Result<CommitArgs, Error> {
    let arguments = arguments.ok_or(Error)?;
    let mut include_tick = None;
    let mut session_id = None;
    for (key, value) in arguments {
        match key.as_str() {
            "include_tick" => include_tick = Some(value.as_bool().ok_or(Error)?),
            "session_id" => session_id = Some(value.as_str().ok_or(Error)?.to_string()),
            _ => return Err(Error),
        }
    }
    Ok(CommitArgs {
        include_tick: include_tick.ok_or(Error)?,
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

pub fn read_say_args(arguments: Option<&Map<String, Value>>) -> Result<SayArgs, Error> {
    let arguments = arguments.ok_or(Error)?;
    let mut address = None;
    let mut place = None;
    let mut actor = None;
    let mut text = None;
    for (key, value) in arguments {
        match key.as_str() {
            "from" => return Err(Error),
            "address" => address = Some(value.as_str().ok_or(Error)?.to_string()),
            "place" => place = optional_text(value)?,
            "actor" => actor = optional_text(value)?,
            "text" => text = Some(value.as_str().ok_or(Error)?.to_string()),
            _ => return Err(Error),
        }
    }
    Ok(SayArgs {
        address: address.ok_or(Error)?,
        text: text.ok_or(Error)?,
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

pub fn read_intent_args(arguments: Option<&Map<String, Value>>) -> Result<IntentCall, Error> {
    let arguments = arguments.ok_or(Error)?;
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
            "id" => id = Some(value.as_str().ok_or(Error)?.to_string()),
            "tick_index" => tick_index = Some(value.as_u64().ok_or(Error)?),
            "actor_id" => actor_id = Some(value.as_str().ok_or(Error)?.to_string()),
            "actor_kind" => actor_kind = Some(value.as_str().ok_or(Error)?.to_string()),
            "source" => source = Some(value.as_str().ok_or(Error)?.to_string()),
            "move" => move_name = Some(value.as_str().ok_or(Error)?.to_string()),
            "target" => target = read_target(value)?,
            "args" => message_to = read_message_to(value)?,
            "rationale_code" => rationale_code = value.as_str().ok_or(Error)?.to_string(),
            _ => return Err(Error),
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_says",
        deserialize_with = "deserialize_says"
    )]
    pub events: Option<Vec<GmSay>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPresence {
    pub humans: u32,
    pub actors_present: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPause {
    pub named_plot_clocks: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roll {
    pub faces: Vec<u32>,
    pub stat: i64,
    pub total: i64,
    pub target: i64,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmSay {
    pub seq: u64,
    pub from: String,
    pub address: String,
    pub place: Option<String>,
    pub actor: Option<String>,
    pub recipients: Vec<String>,
    pub text: String,
}

/// A `spook/event` body. `kind` is the serde tag, not a stored field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EventBody {
    Say(GmSay),
    Queued(QueueRow),
    Check(Roll),
}

impl GmSay {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(EventBody::Say(self.clone())).expect("say")
    }
}

fn serialize_says<S>(events: &Option<Vec<GmSay>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    events
        .as_ref()
        .map(|says| says.iter().cloned().map(EventBody::Say).collect::<Vec<_>>())
        .serialize(serializer)
}

fn deserialize_says<'de, D>(deserializer: D) -> Result<Option<Vec<GmSay>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let events = Vec::<EventBody>::deserialize(deserializer)?;
    events
        .into_iter()
        .map(|event| match event {
            EventBody::Say(say) => Ok(say),
            EventBody::Queued(_) | EventBody::Check(_) => Err(serde::de::Error::custom("say")),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
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

/// Sent after the GM seat is free. The client waits on this result.
pub fn released_result() -> Value {
    serde_json::json!({ "released": true })
}

fn read_target(value: &Value) -> Result<CallTarget, Error> {
    let object = value.as_object().ok_or(Error)?;
    if object.is_empty() {
        return Ok(CallTarget::Empty);
    }
    if object.len() != 1 {
        return Err(Error);
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
    Err(Error)
}

fn read_message_to(value: &Value) -> Result<Option<Vec<String>>, Error> {
    let object = value.as_object().ok_or(Error)?;
    let mut message_to = None;
    for (key, item) in object {
        match key.as_str() {
            "to" => {
                let values = item.as_array().ok_or(Error)?;
                let mut ids = Vec::new();
                for value in values {
                    ids.push(value.as_str().ok_or(Error)?.to_string());
                }
                message_to = Some(ids);
            }
            _ => return Err(Error),
        }
    }
    Ok(message_to)
}

fn optional_text(value: &Value) -> Result<Option<String>, Error> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => Err(Error),
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
