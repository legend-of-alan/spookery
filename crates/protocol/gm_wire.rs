//! JSON lines for the GM seat: tool calls, events, and status.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{Error, Outcome};

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

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusArgs {
    #[serde(default)]
    pub since: Option<u64>,
}

pub fn read_status_args(arguments: Option<&Map<String, Value>>) -> Result<StatusArgs, Error> {
    match arguments {
        None => Ok(StatusArgs::default()),
        Some(arguments) => parse_args(arguments),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerceiveArgs {
    #[serde(rename = "as", default)]
    pub as_viewer: Option<String>,
    #[serde(default, deserialize_with = "deserialize_focus")]
    pub focus: Option<String>,
    #[serde(default)]
    pub budget_tokens: Option<u64>,
}

fn deserialize_focus<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let text = String::deserialize(deserializer)?;
    match text.as_str() {
        "place" | "actor" => Ok(Some(text)),
        _ => Err(serde::de::Error::custom("focus")),
    }
}

pub fn read_perceive_args(arguments: Option<&Map<String, Value>>) -> Result<PerceiveArgs, Error> {
    match arguments {
        None => Ok(PerceiveArgs::default()),
        Some(arguments) => parse_args(arguments),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitArgs {
    pub include_tick: bool,
    #[serde(default)]
    pub session_id: Option<String>,
}

pub fn read_commit_args(arguments: Option<&Map<String, Value>>) -> Result<CommitArgs, Error> {
    parse_args(arguments.ok_or(Error)?)
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SayArgs {
    pub address: String,
    pub text: String,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

pub fn read_say_args(arguments: Option<&Map<String, Value>>) -> Result<SayArgs, Error> {
    parse_args(arguments.ok_or(Error)?)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CallTarget {
    #[default]
    Empty,
    Place(String),
    Actor(String),
    Clock(String),
}

impl<'de> Deserialize<'de> for CallTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireTarget {
            #[serde(default)]
            place: Option<String>,
            #[serde(default)]
            actor: Option<String>,
            #[serde(default)]
            clock: Option<String>,
        }
        let wire = WireTarget::deserialize(deserializer)?;
        match (wire.place, wire.actor, wire.clock) {
            (None, None, None) => Ok(CallTarget::Empty),
            (Some(id), None, None) => Ok(CallTarget::Place(id)),
            (None, Some(id), None) => Ok(CallTarget::Actor(id)),
            (None, None, Some(id)) => Ok(CallTarget::Clock(id)),
            _ => Err(serde::de::Error::custom("target")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub tick_index: Option<u64>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub actor_kind: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(rename = "move", default)]
    pub move_name: Option<String>,
    #[serde(default)]
    pub target: CallTarget,
    #[serde(rename = "args", default, deserialize_with = "deserialize_message_to")]
    pub message_to: Option<Vec<String>>,
    #[serde(default)]
    pub rationale_code: String,
}

fn deserialize_message_to<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireArgs {
        #[serde(default)]
        to: Option<Vec<String>>,
    }
    Ok(WireArgs::deserialize(deserializer)?.to)
}

pub fn read_intent_args(arguments: Option<&Map<String, Value>>) -> Result<IntentCall, Error> {
    parse_args(arguments.ok_or(Error)?)
}

fn parse_args<T: serde::de::DeserializeOwned>(arguments: &Map<String, Value>) -> Result<T, Error> {
    serde_json::from_value(Value::Object(arguments.clone())).map_err(|_| Error)
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
    let body = EventBody::Queued(QueueRow {
        id: id.to_string(),
        actor_id: actor_id.to_string(),
        move_name: move_name.to_string(),
    });
    event_line(serde_json::to_value(body).expect("queued event"))
}

pub fn write_check_event(roll: &Roll) -> Vec<u8> {
    let body = EventBody::Check(roll.clone());
    event_line(serde_json::to_value(body).expect("check event"))
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
