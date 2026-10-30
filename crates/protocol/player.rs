//! JSON lines for the player seat. No GM field, and no roll faces.

use serde::{Deserialize, Serialize};

use crate::{Error, Outcome};

/// Names and prose for one place view. Boxed on the wire so the message stays small.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewScene {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub place_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actor_names: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
}

/// A line on the player socket. No GM field, and no roll faces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PlayerFrame {
    Say {
        from: String,
        address: String,
        text: String,
        #[serde(default, skip_serializing_if = "is_false")]
        replay: bool,
    },
    Queued {
        id: String,
    },
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
    Refused {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<Missing>,
    },
    View {
        viewer: String,
        place: String,
        #[serde(flatten, default)]
        scene: Box<ViewScene>,
        visible_actors: Vec<String>,
        known_facts: Vec<KnownFact>,
        notes: Vec<NoteView>,
        unknown_mentions: Vec<String>,
        clocks: Vec<ClockView>,
        items: Vec<ItemStack>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        meters: Vec<MeterView>,
        habits: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exits: Vec<ExitView>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Missing {
    One(String),
    Many(Vec<String>),
}

impl Missing {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::One(text) => Some(text),
            Self::Many(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitView {
    pub name: String,
    pub to: String,
    pub place_name: String,
    pub minutes: u64,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownFact {
    pub id: String,
    pub text: String,
    pub owner_body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteView {
    pub id: String,
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    pub fact_ids: Vec<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockView {
    pub id: String,
    pub value: i64,
    pub max: i64,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStack {
    pub id: String,
    pub count: u32,
}

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

pub fn emit_player(message: &PlayerFrame) -> String {
    let mut line = serde_json::to_string(message).expect("player line");
    line.push('\n');
    line
}

pub fn parse_player(line: &str) -> Result<PlayerFrame, Error> {
    serde_json::from_str(line).map_err(|_| Error)
}

/// `player:{id}\n`. An empty id, or an id with whitespace, is not a hello.
pub fn emit_player_hello(id: &str) -> String {
    format!("player:{id}\n")
}

pub fn parse_player_hello(line: &str) -> Option<&str> {
    let id = line.strip_prefix("player:")?;
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        None
    } else {
        Some(id)
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
            from: "player:rowan".to_string(),
            address: "table".to_string(),
            text: "hello".to_string(),
            replay: false,
        };
        let line = emit_player(&message);
        assert_eq!(
            line,
            "{\"kind\":\"say\",\"from\":\"player:rowan\",\"address\":\"table\",\"text\":\"hello\"}\n"
        );
        assert_eq!(parse_player(&line).unwrap(), message);
    }

    #[test]
    fn a_check_line_has_no_faces() {
        let line = emit_player(&PlayerFrame::Check {
            outcome: Outcome::Success,
        });
        assert_eq!(line, "{\"kind\":\"check\",\"outcome\":\"success\"}\n");
        assert_eq!(
            parse_player(&line).unwrap(),
            PlayerFrame::Check {
                outcome: Outcome::Success,
            }
        );
        let partial = emit_player(&PlayerFrame::Check {
            outcome: Outcome::Partial,
        });
        assert_eq!(partial, "{\"kind\":\"check\",\"outcome\":\"partial\"}\n");
        let miss = emit_player(&PlayerFrame::Check {
            outcome: Outcome::Miss,
        });
        assert_eq!(miss, "{\"kind\":\"check\",\"outcome\":\"miss\"}\n");
        assert_eq!(
            parse_player(r#"{"kind":"check","outcome":"success","faces":[1]}"#).unwrap(),
            PlayerFrame::Check {
                outcome: Outcome::Success,
            }
        );
        assert!(parse_player(r#"{"kind":"check","success":true}"#).is_err());
    }

    #[test]
    fn a_player_hello_round_trips() {
        let line = emit_player_hello("rowan");
        assert_eq!(line, "player:rowan\n");
        assert_eq!(parse_player_hello(line.trim_end()), Some("rowan"));
        assert_eq!(parse_player_hello("player:"), None);
        assert_eq!(parse_player_hello("player:row an"), None);
    }

    #[test]
    fn a_perceive_frame_ignores_unknown_fields() {
        let line = r#"{"kind":"view","viewer":"rowan","place":"shop","visible_actors":[],"known_facts":[],"notes":[],"unknown_mentions":[],"clocks":[],"items":[{"id":"oil","count":1,"extra":true}],"habits":[],"future":1}"#;
        assert!(matches!(parse_player(line), Ok(PlayerFrame::View { .. })));
    }
}
