//! JSON lines for the player seat. No GM field, and no roll faces.

use serde::{Deserialize, Serialize};

use crate::{Outcome, Queued, Refusal, Speaker};

/// Sent from the table to a player client over the game port.
/// A line on the player socket. No GM field, and no roll faces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PlayerFrame {
    Say {
        from: Speaker,
        address: String,
        text: String,
        #[serde(default, skip_serializing_if = "is_false")]
        replay: bool,
    },
    Queued(Queued),
    Check {
        outcome: Outcome,
    },
    Resume {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        notes: Vec<NoteView>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        clocks: Vec<ClockView>,
    },
    /// Help, look, and map. The text is already the lines to print.
    Lines {
        text: String,
    },
    Refused(Refusal),
    View(Box<View>),
}

/// Sent from the table to a player client as the `view` line and to a GM client as the
/// `perceive` result. Always written `protocol::View`: the engine's `View` is what the
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

impl PlayerFrame {
    /// One JSON line, newline included.
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("player line");
        line.push('\n');
        line
    }

    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }
}

/// `player:{id}\n`, the first line a player client sends the table.
pub fn hello_line(id: &str) -> String {
    format!("{}\n", Speaker::Player(id.to_string()))
}

/// The PC id of a hello. An empty id, an id with whitespace, or another speaker is not one.
pub fn parse_hello(line: &str) -> Option<String> {
    match line.parse() {
        Ok(Speaker::Player(id)) => Some(id),
        _ => None,
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_say_line_round_trips() {
        let message = PlayerFrame::Say {
            from: Speaker::Player("rowan".to_string()),
            address: "table".to_string(),
            text: "hello".to_string(),
            replay: false,
        };
        let line = message.to_line();
        assert_eq!(
            line,
            "{\"kind\":\"say\",\"from\":\"player:rowan\",\"address\":\"table\",\"text\":\"hello\"}\n"
        );
        assert_eq!(PlayerFrame::from_line(&line).unwrap(), message);
        let stranger = r#"{"kind":"say","from":"somebody","address":"table","text":"x"}"#;
        assert!(PlayerFrame::from_line(stranger).is_err());
    }

    #[test]
    fn a_check_line_has_no_faces() {
        let line = PlayerFrame::Check {
            outcome: Outcome::Success,
        }
        .to_line();
        assert_eq!(line, "{\"kind\":\"check\",\"outcome\":\"success\"}\n");
        assert_eq!(
            PlayerFrame::from_line(&line).unwrap(),
            PlayerFrame::Check {
                outcome: Outcome::Success,
            }
        );
        let partial = PlayerFrame::Check {
            outcome: Outcome::Partial,
        }
        .to_line();
        assert_eq!(partial, "{\"kind\":\"check\",\"outcome\":\"partial\"}\n");
        let miss = PlayerFrame::Check {
            outcome: Outcome::Miss,
        }
        .to_line();
        assert_eq!(miss, "{\"kind\":\"check\",\"outcome\":\"miss\"}\n");
        assert_eq!(
            PlayerFrame::from_line(r#"{"kind":"check","outcome":"success","faces":[1]}"#).unwrap(),
            PlayerFrame::Check {
                outcome: Outcome::Success,
            }
        );
        assert!(PlayerFrame::from_line(r#"{"kind":"check","success":true}"#).is_err());
    }

    #[test]
    fn a_player_hello_round_trips() {
        let line = hello_line("rowan");
        assert_eq!(line, "player:rowan\n");
        assert_eq!(parse_hello(line.trim_end()).as_deref(), Some("rowan"));
        assert_eq!(parse_hello("player:"), None);
        assert_eq!(parse_hello("player:row an"), None);
        assert_eq!(parse_hello("gm"), None);
    }

    #[test]
    fn a_perceive_frame_ignores_unknown_fields() {
        let line = r#"{"kind":"view","viewer":"rowan","place":"shop","visible_actors":[],"known_facts":[],"notes":[],"unknown_mentions":[],"clocks":[],"items":[{"id":"oil","count":1,"extra":true}],"habits":[],"future":1}"#;
        assert!(matches!(
            PlayerFrame::from_line(line),
            Ok(PlayerFrame::View(_))
        ));
    }

    #[test]
    fn a_view_line_keeps_the_scene_after_the_place() {
        let line = r#"{"kind":"view","viewer":"rowan","place":"shop","place_name":"Shop","body":"b","actor_names":["Rowan"],"status":"s","visible_actors":["rowan"],"known_facts":[],"notes":[],"unknown_mentions":[],"clocks":[],"items":[],"habits":[]}"#;
        let frame = PlayerFrame::from_line(line).unwrap();
        assert_eq!(frame.to_line(), format!("{line}\n"));
    }
}
