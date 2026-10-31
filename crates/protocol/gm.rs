//! JSON lines for the GM seat: tool calls, events, and status.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{Outcome, Speaker};

/// Deserialize a tool's arguments. `None` is `{}`; the argument types decide what is required.
pub fn parse_args<T: DeserializeOwned>(
    arguments: Option<&Map<String, Value>>,
) -> Result<T, serde_json::Error> {
    serde_json::from_value(Value::Object(arguments.cloned().unwrap_or_default()))
}

/// `table_status` arguments, sent from the GM's MCP client to the table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// `perceive` arguments, sent from the GM's MCP client to the table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerceiveArgs {
    #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
    pub as_viewer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_body_chars: Option<u64>,
}

/// `commit_round` arguments, sent from the GM's MCP client to the table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitArgs {
    pub include_round: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// `close` arguments, sent from the GM's MCP client to the table. No key is accepted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseArgs {}

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

/// `queue_intent` arguments, sent from the GM's MCP client to the table.
/// Serialized, a call names only the fields that are set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(rename = "move", default, skip_serializing_if = "Option::is_none")]
    pub move_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(rename = "to", default, skip_serializing_if = "Option::is_none")]
    pub message_to: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rationale_code: String,
}

/// Sent from the table to a player client as the `queued` line and to a GM client as the
/// `queue_intent` result: the one intent id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queued {
    pub id: String,
}

/// Sent from the table to a GM client as the `close` result, after the GM seat is free.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseReply {
    pub released: bool,
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
    pub from: Speaker,
    pub address: String,
    pub place: Option<String>,
    pub actor: Option<String>,
    pub recipients: Vec<String>,
    pub text: String,
}

/// Whether a `MenuRow` is a PC or an NPC, sent from the table to the GM's MCP client.
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

/// Sent from the table to a GM client as a `spook/event` notification; the same objects
/// come back from `table_status` with `since`, and a `say` result is a `Say`.
/// `kind` is the serde tag, not a stored field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GmEvent {
    Say(SayEvent),
    Queued(QueuedMove),
    Check(Roll),
}

fn serialize_says<S>(events: &Option<Vec<SayEvent>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    events
        .as_ref()
        .map(|says| says.iter().cloned().map(GmEvent::Say).collect::<Vec<_>>())
        .serialize(serializer)
}

fn deserialize_says<'de, D>(deserializer: D) -> Result<Option<Vec<SayEvent>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let events = Vec::<GmEvent>::deserialize(deserializer)?;
    events
        .into_iter()
        .map(|event| match event {
            GmEvent::Say(say) => Ok(say),
            GmEvent::Queued(_) | GmEvent::Check(_) => Err(serde::de::Error::custom("say")),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
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
            assert_reencodes::<GmEvent>(raw);
        }
        let m =
            serde_json::from_str(r#"{"actor_id":"a","move":"m","target":"t","to":["b"]}"#).unwrap();
        let c: IntentCall = parse_args(Some(&m)).unwrap();
        assert_eq!(c.actor_id.as_deref(), Some("a"));
        assert_eq!(c.move_name.as_deref(), Some("m"));
        assert_eq!(c.target.as_deref(), Some("t"));
        assert_eq!(c.message_to, Some(vec!["b".to_string()]));
        let nested =
            serde_json::from_str(r#"{"actor_id":"a","move":"m","args":{"to":["b"]}}"#).unwrap();
        assert!(parse_args::<IntentCall>(Some(&nested)).is_err());
    }

    #[test]
    fn an_intent_call_names_only_the_set_fields() {
        let call = |raw: &str| {
            let args = serde_json::from_str(raw).unwrap();
            serde_json::to_string(&parse_args::<IntentCall>(Some(&args)).unwrap()).unwrap()
        };
        let bare = r#"{"actor_id":"mira","move":"restock_lamp_oil"}"#;
        assert_eq!(call(bare), bare);
        let aimed = r#"{"actor_id":"pc_rowan","move":"travel","target":"docks_street"}"#;
        assert_eq!(call(aimed), aimed);
    }

    #[test]
    fn missing_arguments_are_an_empty_object() {
        let args = |raw: &str| serde_json::from_str::<Map<String, Value>>(raw).unwrap();
        let read =
            parse_args::<PerceiveArgs>(Some(&args(r#"{"as":"pc_rowan","max_body_chars":200}"#)));
        assert_eq!(read.unwrap().max_body_chars, Some(200));
        assert!(
            parse_args::<PerceiveArgs>(Some(&args(r#"{"as":"pc_rowan","focus":"place"}"#)))
                .is_err()
        );
        assert_eq!(
            parse_args::<StatusArgs>(None).unwrap(),
            StatusArgs::default()
        );
        assert!(parse_args::<CommitArgs>(None).is_err());
        assert!(parse_args::<SayArgs>(None).is_err());
        assert!(parse_args::<CloseArgs>(None).is_ok());
        assert!(parse_args::<CloseArgs>(Some(&args(r#"{"x":1}"#))).is_err());
    }
}
