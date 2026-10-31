//! JSON lines for the GM seat: tool calls, events, and status.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{Error, Outcome};

/// The structured body of a tool-result line.
pub fn parse_tool_result(line: &str) -> Result<Value, Error> {
    let value: Value = serde_json::from_str(line).map_err(|_| Error)?;
    let result = value.get("result").ok_or(Error)?;
    Ok(result
        .get("structuredContent")
        .or_else(|| result.get("structured_content"))
        .cloned()
        .unwrap_or(Value::Null))
}

/// `table_status` arguments, sent from the GM's MCP client to the table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusArgs {
    #[serde(default)]
    pub since: Option<u64>,
}

pub fn parse_status_args(arguments: Option<&Map<String, Value>>) -> Result<StatusArgs, Error> {
    match arguments {
        None => Ok(StatusArgs::default()),
        Some(arguments) => parse_args(arguments),
    }
}

/// `perceive` arguments, sent from the GM's MCP client to the table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerceiveArgs {
    #[serde(rename = "as", default)]
    pub as_viewer: Option<String>,
    #[serde(default)]
    pub max_body_chars: Option<u64>,
}

pub fn parse_perceive_args(arguments: Option<&Map<String, Value>>) -> Result<PerceiveArgs, Error> {
    match arguments {
        None => Ok(PerceiveArgs::default()),
        Some(arguments) => parse_args(arguments),
    }
}

/// `commit_round` arguments, sent from the GM's MCP client to the table.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitArgs {
    pub include_round: bool,
    #[serde(default)]
    pub session_id: Option<String>,
}

pub fn parse_commit_args(arguments: Option<&Map<String, Value>>) -> Result<CommitArgs, Error> {
    parse_args(arguments.ok_or(Error)?)
}

/// The raw `say` arguments from the GM's MCP client, before `SayArgs` checks them.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SayWire {
    address: String,
    text: String,
    #[serde(default)]
    place: Option<String>,
    #[serde(default)]
    actor: Option<String>,
}

/// `say` arguments, sent from the GM's MCP client to the table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "address", rename_all = "lowercase", try_from = "SayWire")]
pub enum SayArgs {
    Table { text: String },
    Place { place: String, text: String },
    Actor { actor: String, text: String },
}

impl SayArgs {
    pub fn text(&self) -> &str {
        match self {
            SayArgs::Table { text } | SayArgs::Place { text, .. } | SayArgs::Actor { text, .. } => {
                text
            }
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("say arguments")
    }
}

impl TryFrom<SayWire> for SayArgs {
    type Error = &'static str;

    fn try_from(args: SayWire) -> Result<Self, Self::Error> {
        match (args.address.as_str(), args.place, args.actor) {
            ("table", None, None) => Ok(SayArgs::Table { text: args.text }),
            ("place", Some(place), None) if !place.is_empty() => Ok(SayArgs::Place {
                place,
                text: args.text,
            }),
            ("actor", None, Some(actor)) if !actor.is_empty() => Ok(SayArgs::Actor {
                actor,
                text: args.text,
            }),
            _ => Err("say"),
        }
    }
}

pub fn parse_say_args(arguments: Option<&Map<String, Value>>) -> Result<SayArgs, Error> {
    parse_args(arguments.ok_or(Error)?)
}

/// `queue_intent` arguments, sent from the GM's MCP client to the table.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub round_index: Option<u64>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(rename = "move", default)]
    pub move_name: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(rename = "to", default)]
    pub message_to: Option<Vec<String>>,
    #[serde(default)]
    pub rationale_code: String,
}

pub fn parse_intent_args(arguments: Option<&Map<String, Value>>) -> Result<IntentCall, Error> {
    parse_args(arguments.ok_or(Error)?)
}

fn parse_args<T: serde::de::DeserializeOwned>(arguments: &Map<String, Value>) -> Result<T, Error> {
    serde_json::from_value(Value::Object(arguments.clone())).map_err(|_| Error)
}

/// The `table_status` result, sent from the table to the GM's MCP client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableStatus {
    pub presence: StatusPresence,
    pub pause: StatusPause,
    pub world_time: u64,
    pub round_index: u64,
    pub state_rev: u64,
    pub pack_rev: String,
    pub last_round_at: String,
    pub queued: Vec<QueuedMove>,
    pub last_seq: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_says",
        deserialize_with = "deserialize_says"
    )]
    pub events: Option<Vec<SayEvent>>,
    /// Sent when the call has no `since`. An older server never sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<PackSummary>,
}

/// Part of `TableStatus`, sent from the table to the GM's MCP client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPresence {
    pub players: u32,
    pub actors_present: Vec<String>,
}

/// Part of `TableStatus`, sent from the table to the GM's MCP client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPause {
    pub presence_clocks: String,
}

/// A queued player move, sent from the table to the GM's MCP client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedMove {
    pub id: String,
    pub actor_id: String,
    #[serde(rename = "move")]
    pub move_name: String,
}

impl TableStatus {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("table status")
    }
}

/// The `commit_round` result, sent from the table to the GM's MCP client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitBody {
    pub state_rev: u64,
    pub changed_keys: Vec<String>,
    pub journal_path: Option<String>,
    pub rolls: Vec<Roll>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dice: Vec<String>,
}

impl CommitBody {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("commit")
    }
}

/// One check roll, sent from the table to the GM's MCP client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roll {
    pub faces: Vec<u32>,
    pub stat: i64,
    pub total: i64,
    pub target: i64,
    pub outcome: Outcome,
}

/// A `say` event or result, sent from the table to the GM's MCP client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SayEvent {
    pub seq: u64,
    pub from: String,
    pub address: String,
    pub place: Option<String>,
    pub actor: Option<String>,
    pub recipients: Vec<String>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CastKind {
    Npc,
    PcTemplate,
}

/// One cast row in a `PackSummary`, sent from the table to the GM's MCP client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuRow {
    pub name: String,
    pub kind: String,
    pub target: String,
}

/// Part of `TableStatus`, sent from the table to the GM's MCP client.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackSummary {
    pub menu: Vec<MenuRow>,
    pub kinds: std::collections::BTreeMap<String, CastKind>,
    pub places: std::collections::BTreeMap<String, String>,
    pub cast: std::collections::BTreeMap<String, String>,
}

/// Sent from the table to the GM's MCP client as a `spook/event` notification.
/// A `spook/event` body. `kind` is the serde tag, not a stored field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EventBody {
    Say(SayEvent),
    Queued(QueuedMove),
    Check(Roll),
}

impl SayEvent {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(EventBody::Say(self.clone())).expect("say")
    }
}

fn serialize_says<S>(events: &Option<Vec<SayEvent>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    events
        .as_ref()
        .map(|says| says.iter().cloned().map(EventBody::Say).collect::<Vec<_>>())
        .serialize(serializer)
}

fn deserialize_says<'de, D>(deserializer: D) -> Result<Option<Vec<SayEvent>>, D::Error>
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

pub fn queued_result(id: &str) -> Value {
    serde_json::json!({ "id": id })
}

pub fn code_result(code: &str) -> Value {
    serde_json::json!({ "code": code })
}

/// Sent after the GM seat is free. The client waits on this result.
pub fn released_result() -> Value {
    serde_json::json!({ "released": true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gm_frames_ignore_unknown_fields_and_round_trip() {
        fn assert_reencodes<
            T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
        >(
            s: &str,
        ) {
            let v: T = serde_json::from_str(s).unwrap();
            let back: T = serde_json::from_value(serde_json::to_value(&v).unwrap()).unwrap();
            assert_eq!(back, v);
        }
        assert_reencodes::<TableStatus>(
            r#"{"presence":{"players":1,"actors_present":["pc_rowan"],"x":1},"pause":{"presence_clocks":"running","x":1},"world_time":10,"round_index":1,"state_rev":2,"pack_rev":"p","last_round_at":"t","queued":[{"id":"q1","actor_id":"pc_rowan","move":"wait","x":1}],"last_seq":3,"pack":{"menu":[],"kinds":{},"places":{},"cast":{},"x":1},"x":1}"#,
        );
        assert_reencodes::<CommitBody>(
            r#"{"state_rev":3,"changed_keys":["d"],"journal_path":null,"rolls":[{"faces":[3,4],"stat":1,"total":8,"target":7,"outcome":"partial","x":1}],"x":1}"#,
        );
        for raw in [
            r#"{"kind":"say","seq":1,"from":"gm","address":"table","place":null,"actor":null,"recipients":[],"text":"hi","x":1}"#,
            r#"{"kind":"queued","id":"q1","actor_id":"pc_rowan","move":"wait","x":1}"#,
            r#"{"kind":"check","faces":[6,6],"stat":0,"total":12,"target":10,"outcome":"success","x":1}"#,
        ] {
            assert_reencodes::<EventBody>(raw);
        }
        let m =
            serde_json::from_str(r#"{"actor_id":"a","move":"m","target":"t","to":["b"]}"#).unwrap();
        let c = parse_intent_args(Some(&m)).unwrap();
        assert_eq!(c.actor_id.as_deref(), Some("a"));
        assert_eq!(c.move_name.as_deref(), Some("m"));
        assert_eq!(c.target.as_deref(), Some("t"));
        assert_eq!(c.message_to, Some(vec!["b".to_string()]));
        let nested =
            serde_json::from_str(r#"{"actor_id":"a","move":"m","args":{"to":["b"]}}"#).unwrap();
        assert!(parse_intent_args(Some(&nested)).is_err());
    }

    #[test]
    fn perceive_takes_as_and_max_body_chars_only() {
        let args = |raw: &str| serde_json::from_str::<Map<String, Value>>(raw).unwrap();
        let read = parse_perceive_args(Some(&args(r#"{"as":"pc_rowan","max_body_chars":200}"#)));
        assert_eq!(read.unwrap().max_body_chars, Some(200));
        assert!(parse_perceive_args(Some(&args(r#"{"as":"pc_rowan","focus":"place"}"#))).is_err());
    }
}
