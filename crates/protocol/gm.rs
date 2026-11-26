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

/// The GM's `perceive` command arguments, sent from a GM client to the host.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PerceiveArgs {
    /// The actor whose view the GM reads. The schema requires it, and the host refuses a call
    /// without it as `UNSCOPED_PERCEIVE`.
    #[cfg_attr(feature = "schema", schemars(required))]
    #[serde(rename = "as", skip_serializing_if = "Option::is_none")]
    pub viewer_id: Option<String>,
}

/// The GM's `commit` command arguments, sent from a GM client to the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CommitArgs {
    pub include_round: bool,
    #[serde(rename = "session", default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Who hears a say: everyone at the table, everyone in one place, or one actor. On the wire
/// it is `"table"`, `{"place": "<id>"}` or `{"actor": "<id>"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(inline))]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    Table,
    Place(String),
    Actor(String),
}

/// The `say` command arguments of either seat, sent from a client to the host. Only the GM
/// may set `as`, the bare id of an NPC to speak as, and may then leave out `to`: the NPC's
/// place hears it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SayArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Audience>,
    pub text: String,
    #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
    pub npc_id: Option<String>,
}

/// The GM's `queue` command arguments, sent from a GM client to the host.
/// Serialized, a call names only the fields that are set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct QueueArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_index: Option<u64>,
    /// The actor the verb is for. The schema requires it, and the host refuses a call without
    /// it as `SCHEMA`.
    #[cfg_attr(feature = "schema", schemars(required))]
    #[serde(rename = "actor", skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    /// The verb to queue. The schema requires it, and the host refuses a call without it as
    /// `SCHEMA`.
    #[cfg_attr(feature = "schema", schemars(required))]
    #[serde(rename = "verb", skip_serializing_if = "Option::is_none")]
    pub verb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(rename = "to", default, skip_serializing_if = "Option::is_none")]
    pub message_to: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rationale_code: String,
}

/// The body of the GM's `status` reply, sent from the host to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HostStatus {
    pub presence: StatusPresence,
    pub world_time: u64,
    pub round_index: u64,
    pub state_rev: u64,
    pub pack_rev: String,
    pub last_round_at: String,
    pub queued: Vec<QueuedVerb>,
    pub pack: PackSummary,
}

/// Part of `HostStatus`, sent from the host to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StatusPresence {
    pub players: u32,
    pub actors_present: Vec<String>,
}

/// A queued verb in `HostStatus`, sent from the host to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QueuedVerb {
    pub id: String,
    #[serde(rename = "actor")]
    pub actor_id: String,
    #[serde(rename = "verb")]
    pub verb_id: String,
}

/// The body of the GM's `committed` reply, sent from the host to a GM client.
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

/// One check roll in `CommitBody`, sent from the host to a GM client. Its dice are in the
/// body's `dice` lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Roll {
    pub total: i64,
    pub success_at: i64,
    pub outcome: Outcome,
}

/// Whether a cast entry is an NPC or a PC template, sent from the host to a GM client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ControlType {
    Npc,
    PcTemplate,
}

/// One cast row in a `PackSummary`, sent from the host to a GM client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MenuRow {
    pub id: String,
    pub kind: String,
    pub target: String,
}

/// Part of `HostStatus`, sent from the host to a GM client.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PackSummary {
    pub menu: Vec<MenuRow>,
    pub control_types: std::collections::BTreeMap<String, ControlType>,
    pub places: std::collections::BTreeMap<String, String>,
    pub cast: std::collections::BTreeMap<String, String>,
    /// The pack's tone and flavor for the GM, `gm_notes` in `pack.yaml` as written. Left out when
    /// the pack has none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gm_notes: String,
}

/// The structured content of an `/mcp` tool result, sent from the host to an MCP GM client:
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
        assert_reencodes::<HostStatus>(
            r#"{"presence":{"players":1,"actors_present":["rowan"],"x":1},"world_time":10,"round_index":1,"state_rev":2,"pack_rev":"p","last_round_at":"t","queued":[{"id":"q1","actor":"rowan","verb":"wait","x":1}],"pack":{"menu":[],"control_types":{},"places":{},"cast":{},"x":1},"x":1}"#,
        );
        assert_reencodes::<CommitBody>(
            r#"{"state_rev":3,"changed_keys":["d"],"journal_path":null,"rolls":[{"total":8,"success_at":7,"outcome":"partial","x":1}],"x":1}"#,
        );
        let m =
            serde_json::from_str(r#"{"actor":"a","verb":"m","target":"t","to":["b"]}"#).unwrap();
        let c: QueueArgs = parse_args(Some(&m)).unwrap();
        assert_eq!(c.actor_id.as_deref(), Some("a"));
        assert_eq!(c.verb_id.as_deref(), Some("m"));
        assert_eq!(c.target.as_deref(), Some("t"));
        assert_eq!(c.message_to, Some(vec!["b".to_string()]));
        let nested =
            serde_json::from_str(r#"{"actor":"a","verb":"m","args":{"to":["b"]}}"#).unwrap();
        assert!(parse_args::<QueueArgs>(Some(&nested)).is_err());
    }

    #[test]
    fn an_intent_call_names_only_the_set_fields() {
        let call = |raw: &str| {
            let args = serde_json::from_str(raw).unwrap();
            serde_json::to_string(&parse_args::<QueueArgs>(Some(&args)).unwrap()).unwrap()
        };
        let bare = r#"{"actor":"mira","verb":"restock_lamp_oil"}"#;
        assert_eq!(call(bare), bare);
        let aimed = r#"{"actor":"rowan","verb":"travel","target":"docks_street"}"#;
        assert_eq!(call(aimed), aimed);
    }

    #[test]
    fn missing_arguments_are_an_empty_object() {
        let args = |raw: &str| serde_json::from_str::<Map<String, Value>>(raw).unwrap();
        let read = parse_args::<PerceiveArgs>(Some(&args(r#"{"as":"rowan"}"#)));
        assert_eq!(read.unwrap().viewer_id.as_deref(), Some("rowan"));
        assert!(
            parse_args::<PerceiveArgs>(Some(&args(r#"{"as":"rowan","focus":"place"}"#))).is_err()
        );
        assert!(parse_args::<CommitArgs>(None).is_err());
        assert!(parse_args::<SayArgs>(None).is_err());
    }
}
