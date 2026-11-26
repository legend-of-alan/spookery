//! The view a player seat sees, also the GM's `perceive` reply.

use serde::{Deserialize, Serialize};

/// Sent from the host to a player client as the `view` body and to a GM client as the
/// `perceive` reply. Always written `protocol::View`: the engine's `View` is what the
/// server renders this from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct View {
    pub viewer: String,
    /// Where the viewer is, with the exits they see there. A walker's `place` is
    /// `path:<id>` and its name the path's.
    pub place: PlaceView,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actor_names: Vec<String>,
    /// Why the viewer can't send a move at all. Empty when they can, and empty today: being
    /// busy is `busy`, not this.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cant_act: String,
    /// What keeps the viewer busy and until when, such as `on the stairs until tick 1300`.
    /// Empty when they are free.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub busy: String,
    pub visible_actors: Vec<String>,
    pub notes: Vec<NoteView>,
    /// The items lying in the viewer's place.
    pub items: Vec<ItemStack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items_carried: Vec<ItemStack>,
    /// Which of `items_carried` the viewer wields.
    #[serde(
        rename = "wielded_item",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub wielded_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meters: Vec<MeterView>,
    /// The world's meters this viewer may see, at their current values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub world_meters: Vec<MeterView>,
    pub habits: Vec<String>,
    /// The places `place.exits` go to that the viewer's `map` knows, each with the exits the
    /// viewer saw there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nearby: Vec<PlaceView>,
}

impl View {
    /// Whether the viewer can send a move at all. A busy viewer can, and is refused `BUSY`.
    pub fn can_act(&self) -> bool {
        self.cant_act.is_empty()
    }
}

/// Part of a `View`, sent from the host to a player client. `path` is the path it walks, as
/// a verb's `path` target, a `path_flag` change and a walker's `path:<id>` name it. `name` is
/// what a player reads for the exit, such as `stairs` or `Docks Street`. `place` and
/// `place_name` are where it goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExitView {
    #[serde(rename = "path")]
    pub path_id: String,
    pub name: String,
    #[serde(rename = "place")]
    pub place_id: String,
    pub place_name: String,
    pub ticks: u64,
    pub locked: bool,
}

/// Part of a `View`, sent from the host to a player client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct NoteView {
    pub id: String,
    #[serde(rename = "to")]
    pub to_ids: Vec<String>,
    #[serde(rename = "place", default, skip_serializing_if = "Option::is_none")]
    pub place_id: Option<String>,
    pub text: String,
}

/// Part of a `View`, sent from the host to a player client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemStack {
    pub id: String,
    pub count: u32,
    /// The item's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// Part of a `View`, sent from the host to a player client. `name` is the pack's name for the
/// meter, left out when it gives none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MeterView {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub value: i64,
    pub max: i64,
}

/// One change of a `changes` body, sent from the host to a player client. `component`
/// names what changed, such as `meter`, `item` or `place`. `entity` is whose it is as a
/// rule's `of` names it: an actor id, `place:<id>`, `story:<id>` or `world`.
/// `entity_name` is that actor's, place's or story's name. `entry` is which one of the
/// entity's, such as the meter or item id, and `entry_name` the item's or meter's name, or for
/// `place` the name of the place moved to. `before` and `after` are left out when there was none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ChangeView {
    #[serde(rename = "component")]
    pub component_id: String,
    #[serde(rename = "entity")]
    pub entity_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entity_name: String,
    #[serde(rename = "entry", default, skip_serializing_if = "String::is_empty")]
    pub entry_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entry_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<ChangeValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<ChangeValue>,
}

/// One side of a `ChangeView`, written as the JSON value it is: a number, text, a mark or a list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum ChangeValue {
    Number(i64),
    Text(String),
    Bool(bool),
    List(Vec<String>),
}

impl ChangeValue {
    /// The text, when this is text.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ChangeValue::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The number, when this is a number.
    pub fn as_number(&self) -> Option<i64> {
        match self {
            ChangeValue::Number(number) => Some(*number),
            _ => None,
        }
    }

    /// Whether this is a list that holds `id`.
    pub fn contains(&self, id: &str) -> bool {
        matches!(self, ChangeValue::List(ids) if ids.iter().any(|listed| listed == id))
    }
}

/// One command of a `help` reply. `id` is the command, as a player types it. `target` is what
/// it takes: `none`, `actor`, `place`, `path` or `item`. `names` lists what the player can name
/// for it now, by the names they read. `owner` is the id of the cast member or item whose verb
/// this is, reached by naming them, and then `names` holds its name alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CommandView {
    pub id: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

/// Part of a `View`, as its `place` and each of `nearby`, and of a `map` reply, sent from the
/// host to a client: its id, its name and the exits the viewer may see there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlaceView {
    #[serde(rename = "place")]
    pub place_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exits: Vec<ExitView>,
}

/// Whether `text` is `candidate`, ignoring case and outer spaces.
pub fn is_named(text: &str, candidate: &str) -> bool {
    text.trim().to_lowercase() == candidate.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use crate::ServerBody;

    #[test]
    fn a_view_body_ignores_unknown_fields() {
        let body = r#"{"kind":"view","viewer":"rowan","place":{"place":"shop","name":"Shop"},"visible_actors":[],"notes":[],"items":[{"id":"oil","count":1,"extra":true}],"habits":[],"future":1}"#;
        assert!(matches!(
            serde_json::from_str(body),
            Ok(ServerBody::View(_))
        ));
    }

    #[test]
    fn a_view_body_keeps_the_scene_after_the_place() {
        let body = r#"{"kind":"view","viewer":"rowan","place":{"place":"shop","name":"Shop"},"body":"b","actor_names":["Rowan"],"cant_act":"s","visible_actors":["rowan"],"notes":[],"items":[],"habits":[]}"#;
        let view: ServerBody = serde_json::from_str(body).unwrap();
        assert_eq!(serde_json::to_string(&view).unwrap(), body);
    }
}
