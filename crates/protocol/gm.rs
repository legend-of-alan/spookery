//! The GM seat's command arguments, the bodies of its `status` and `committed` replies, and the
//! result `/mcp` answers a tool call with.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{Outcome, ServerMessage};

/// Deserialize a tool's arguments. `None` is `{}`; the argument types decide what is required.
pub fn parse_args<T: DeserializeOwned>(
    arguments: Option<&Map<String, Value>>,
) -> Result<T, serde_json::Error> {
    serde_json::from_value(Value::Object(arguments.cloned().unwrap_or_default()))
}

/// The GM's `perceive` command arguments, sent from a GM client to the table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PerceiveArgs {
    // An `Option` so the table can refuse a missing `as` with its own code; a missing
    // field reads as `None` without `default`, and the schema says it is required.
    #[cfg_attr(feature = "schema", schemars(required))]
    #[serde(rename = "as", skip_serializing_if = "Option::is_none")]
    pub as_viewer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_body_chars: Option<u64>,
}

/// The GM's `commit` command arguments, sent from a GM client to the table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CommitArgs {
    pub include_round: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// The raw `say` arguments from a client, before `SayArgs` checks them.
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

/// The `say` command arguments of either seat, sent from a client to the table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
// The schema shows the three tagged shapes, closed and with non-empty names as `SayWire`
// checks them, and an object at its root, which MCP wants.
#[cfg_attr(feature = "schema", schemars(!try_from, deny_unknown_fields, extend("type" = "object")))]
#[serde(tag = "address", rename_all = "lowercase", try_from = "SayWire")]
pub enum SayArgs {
    Table {
        text: String,
    },
    Place {
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        place: String,
        text: String,
    },
    Actor {
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        actor: String,
        text: String,
    },
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

/// The GM's `queue` command arguments, sent from a GM client to the table.
/// Serialized, a call names only the fields that are set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct QueueArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_index: Option<u64>,
    // `Option`s so the table can refuse a missing actor or move as `SCHEMA` itself; a
    // missing field reads as `None` without `default`, and the schema says both are required.
    #[cfg_attr(feature = "schema", schemars(required))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[cfg_attr(feature = "schema", schemars(required))]
    #[serde(rename = "move", skip_serializing_if = "Option::is_none")]
    pub move_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(rename = "to", default, skip_serializing_if = "Option::is_none")]
    pub message_to: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rationale_code: String,
}

/// The body of the GM's `status` reply, sent from the table to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TableStatus {
    pub presence: StatusPresence,
    pub world_time: u64,
    pub round_index: u64,
    pub state_rev: u64,
    pub pack_rev: String,
    pub last_round_at: String,
    pub queued: Vec<QueuedMove>,
    pub pack: PackSummary,
}

/// Part of `TableStatus`, sent from the table to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StatusPresence {
    pub players: u32,
    pub actors_present: Vec<String>,
}

/// A queued move in `TableStatus`, sent from the table to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QueuedMove {
    pub id: String,
    pub actor_id: String,
    #[serde(rename = "move")]
    pub move_name: String,
}

/// The body of the GM's `committed` reply, sent from the table to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CommitBody {
    pub state_rev: u64,
    pub changed_keys: Vec<String>,
    pub journal_path: Option<String>,
    pub rolls: Vec<Roll>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dice: Vec<String>,
}

/// One check roll in `CommitBody`, sent from the table to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Roll {
    pub faces: Vec<u32>,
    pub stat: i64,
    pub total: i64,
    pub target: i64,
    pub outcome: Outcome,
}

/// Whether a `MenuRow` is a PC or an NPC, sent from the table to a GM client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CastKind {
    Npc,
    PcTemplate,
}

/// One cast row in a `PackSummary`, sent from the table to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MenuRow {
    pub name: String,
    pub kind: String,
    pub target: String,
}

/// Part of `TableStatus`, sent from the table to a GM client.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PackSummary {
    pub menu: Vec<MenuRow>,
    pub kinds: std::collections::BTreeMap<String, CastKind>,
    pub places: std::collections::BTreeMap<String, String>,
    pub cast: std::collections::BTreeMap<String, String>,
}

/// The structured content of an `/mcp` tool result, sent from the table to an MCP GM client:
/// every message the call's command sent this seat, in order, its reply last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub messages: Vec<ServerMessage>,
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
            r#"{"presence":{"players":1,"actors_present":["pc_rowan"],"x":1},"world_time":10,"round_index":1,"state_rev":2,"pack_rev":"p","last_round_at":"t","queued":[{"id":"q1","actor_id":"pc_rowan","move":"wait","x":1}],"pack":{"menu":[],"kinds":{},"places":{},"cast":{},"x":1},"x":1}"#,
        );
        assert_reencodes::<CommitBody>(
            r#"{"state_rev":3,"changed_keys":["d"],"journal_path":null,"rolls":[{"faces":[3,4],"stat":1,"total":8,"target":7,"outcome":"partial","x":1}],"x":1}"#,
        );
        let m =
            serde_json::from_str(r#"{"actor_id":"a","move":"m","target":"t","to":["b"]}"#).unwrap();
        let c: QueueArgs = parse_args(Some(&m)).unwrap();
        assert_eq!(c.actor_id.as_deref(), Some("a"));
        assert_eq!(c.move_name.as_deref(), Some("m"));
        assert_eq!(c.target.as_deref(), Some("t"));
        assert_eq!(c.message_to, Some(vec!["b".to_string()]));
        let nested =
            serde_json::from_str(r#"{"actor_id":"a","move":"m","args":{"to":["b"]}}"#).unwrap();
        assert!(parse_args::<QueueArgs>(Some(&nested)).is_err());
    }

    #[test]
    fn an_intent_call_names_only_the_set_fields() {
        let call = |raw: &str| {
            let args = serde_json::from_str(raw).unwrap();
            serde_json::to_string(&parse_args::<QueueArgs>(Some(&args)).unwrap()).unwrap()
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
        assert!(parse_args::<CommitArgs>(None).is_err());
        assert!(parse_args::<SayArgs>(None).is_err());
    }
}
