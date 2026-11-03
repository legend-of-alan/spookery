//! The view a player seat sees, also the GM's `perceive` reply.

use serde::{Deserialize, Serialize};

/// Sent from the table to a player client as the `view` body and to a GM client as the
/// `perceive` reply. Always written `protocol::View`: the engine's `View` is what the
/// server renders this from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub viewer: String,
    pub place: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub place_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actor_names: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    pub visible_actors: Vec<String>,
    pub known_facts: Vec<KnownFact>,
    pub notes: Vec<NoteView>,
    pub unknown_mentions: Vec<String>,
    pub clocks: Vec<ClockView>,
    pub items: Vec<ItemStack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meters: Vec<MeterView>,
    pub habits: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exits: Vec<ExitView>,
}

/// Part of a `View`, sent from the table to a player client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitView {
    pub name: String,
    pub to: String,
    pub place_name: String,
    pub minutes: u64,
    pub locked: bool,
}

/// Part of a `View`, sent from the table to a player client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownFact {
    pub id: String,
    pub text: String,
    pub owner_body: String,
}

/// Part of a `View`, sent from the table to a player client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteView {
    pub id: String,
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    pub fact_ids: Vec<String>,
    pub text: String,
}

/// Part of a `View`, sent from the table to a player client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockView {
    pub id: String,
    pub value: i64,
    pub max: i64,
    pub paused: bool,
}

/// Part of a `View`, sent from the table to a player client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStack {
    pub id: String,
    pub count: u32,
}

/// Part of a `View`, sent from the table to a player client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterView {
    pub id: String,
    pub value: i64,
    pub max: i64,
}

pub fn exit_phrase(name: &str, minutes: u64, locked: bool) -> String {
    if locked {
        format!("{name} (locked) — {minutes} min")
    } else {
        format!("{name} — {minutes} min")
    }
}

#[cfg(test)]
mod tests {
    use crate::ServerBody;

    #[test]
    fn a_view_body_ignores_unknown_fields() {
        let body = r#"{"kind":"view","viewer":"rowan","place":"shop","visible_actors":[],"known_facts":[],"notes":[],"unknown_mentions":[],"clocks":[],"items":[{"id":"oil","count":1,"extra":true}],"habits":[],"future":1}"#;
        assert!(matches!(
            serde_json::from_str(body),
            Ok(ServerBody::View(_))
        ));
    }

    #[test]
    fn a_view_body_keeps_the_scene_after_the_place() {
        let body = r#"{"kind":"view","viewer":"rowan","place":"shop","place_name":"Shop","body":"b","actor_names":["Rowan"],"status":"s","visible_actors":["rowan"],"known_facts":[],"notes":[],"unknown_mentions":[],"clocks":[],"items":[],"habits":[]}"#;
        let view: ServerBody = serde_json::from_str(body).unwrap();
        assert_eq!(serde_json::to_string(&view).unwrap(), body);
    }
}
