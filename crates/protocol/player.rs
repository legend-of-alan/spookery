//! JSON lines for the player seat. No GM field, and no roll faces.

use serde::{Deserialize, Serialize};

use crate::{Error, Outcome};

/// Names and prose for one place view. Boxed on the wire so the message stays small.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewScene {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub place_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prose: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actor_names: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
}

/// A line on the player socket. No GM field, and no roll faces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PlayerMessage {
    Say {
        from: String,
        address: String,
        text: String,
        #[serde(default, skip_serializing_if = "skip_false")]
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
        messages: Vec<WireMessage>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        clocks: Vec<WireClock>,
    },
    /// Help, look, and map. The text is already the lines to print.
    Lines {
        text: String,
    },
    Refused {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missing: Option<Missing>,
    },
    #[serde(rename = "perceive")]
    View {
        viewer: String,
        place: String,
        #[serde(flatten, default)]
        scene: Box<ViewScene>,
        visible_actors: Vec<String>,
        known_facts: Vec<WireFact>,
        messages: Vec<WireMessage>,
        unknown_mentioned: Vec<String>,
        clocks: Vec<WireClock>,
        items: Vec<WireItem>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        meters: Vec<WireMeter>,
        habits: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exits: Vec<WireExit>,
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
pub struct WireExit {
    pub name: String,
    pub to: String,
    pub place_name: String,
    pub minutes: u64,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireFact {
    pub id: String,
    pub text: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireMessage {
    pub id: String,
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    pub fact_ids: Vec<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireClock {
    pub id: String,
    pub value: i64,
    pub max: i64,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireItem {
    pub id: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireMeter {
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

pub fn write_player(message: &PlayerMessage) -> String {
    let mut line = serde_json::to_string(message).expect("player line");
    line.push('\n');
    line
}

pub fn read_player(line: &str) -> Result<PlayerMessage, Error> {
    serde_json::from_str(trim_line(line)).map_err(|_| Error)
}

/// `player:{id}\n`. An empty id, or an id with whitespace, is not a hello.
pub fn write_player_hello(id: &str) -> String {
    format!("player:{id}\n")
}

pub fn read_player_hello(line: &str) -> Option<&str> {
    let id = line.strip_prefix("player:")?;
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        None
    } else {
        Some(id)
    }
}

fn trim_line(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

fn skip_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_say_line_round_trips() {
        let message = PlayerMessage::Say {
            from: "player:rowan".to_string(),
            address: "table".to_string(),
            text: "hello".to_string(),
            replay: false,
        };
        let line = write_player(&message);
        assert_eq!(
            line,
            "{\"kind\":\"say\",\"from\":\"player:rowan\",\"address\":\"table\",\"text\":\"hello\"}\n"
        );
        assert_eq!(read_player(&line).unwrap(), message);
    }

    #[test]
    fn a_check_line_has_no_faces() {
        let line = write_player(&PlayerMessage::Check {
            outcome: Outcome::Success,
        });
        assert_eq!(line, "{\"kind\":\"check\",\"outcome\":\"success\"}\n");
        assert_eq!(
            read_player(&line).unwrap(),
            PlayerMessage::Check {
                outcome: Outcome::Success,
            }
        );
        let partial = write_player(&PlayerMessage::Check {
            outcome: Outcome::Partial,
        });
        assert_eq!(partial, "{\"kind\":\"check\",\"outcome\":\"partial\"}\n");
        let miss = write_player(&PlayerMessage::Check {
            outcome: Outcome::Miss,
        });
        assert_eq!(miss, "{\"kind\":\"check\",\"outcome\":\"miss\"}\n");
        assert_eq!(
            read_player(r#"{"kind":"check","outcome":"success","faces":[1]}"#).unwrap(),
            PlayerMessage::Check {
                outcome: Outcome::Success,
            }
        );
        assert!(read_player(r#"{"kind":"check","success":true}"#).is_err());
    }

    #[test]
    fn a_player_hello_round_trips() {
        let line = write_player_hello("rowan");
        assert_eq!(line, "player:rowan\n");
        assert_eq!(read_player_hello(trim_line(&line)), Some("rowan"));
        assert_eq!(read_player_hello("player:"), None);
        assert_eq!(read_player_hello("player:row an"), None);
    }

    #[test]
    fn a_perceive_frame_ignores_unknown_fields() {
        let line = r#"{"kind":"perceive","viewer":"rowan","place":"shop","visible_actors":[],"known_facts":[],"messages":[],"unknown_mentioned":[],"clocks":[],"items":[{"id":"oil","count":1,"extra":true}],"habits":[],"future":1}"#;
        assert!(matches!(read_player(line), Ok(PlayerMessage::View { .. })));
    }
}
