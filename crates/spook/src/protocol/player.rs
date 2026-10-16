use serde::{Deserialize, Serialize};

use crate::protocol::{Outcome, ProtocolError};

/// Names and prose for one place view. Boxed on the wire so the message stays small.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shown {
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
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum PlayerMessage {
    Say {
        from: String,
        address: String,
        text: String,
    },
    Queued {
        id: String,
    },
    Check {
        outcome: Outcome,
    },
    Resume {
        text: String,
    },
    /// Help, look, and map. The text is already the lines to print.
    Lines {
        text: String,
    },
    Refused {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missing: Option<String>,
    },
    #[serde(rename = "perceive")]
    View {
        viewer: String,
        place: String,
        #[serde(flatten, default)]
        shown: Box<Shown>,
        visible_actors: Vec<String>,
        known_facts: Vec<WireFact>,
        messages: Vec<FactNote>,
        unknown_mentioned: Vec<String>,
        clocks: Vec<WireClock>,
        items: Vec<WireItem>,
        habits: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exits: Vec<WireExit>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireExit {
    pub name: String,
    pub to: String,
    pub place_name: String,
    pub minutes: u64,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireFact {
    pub id: String,
    pub text: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactNote {
    pub id: String,
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    pub fact_ids: Vec<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireClock {
    pub id: String,
    pub value: i64,
    pub max: i64,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireItem {
    pub id: String,
    pub count: u32,
}

pub fn write_player(message: &PlayerMessage) -> String {
    let mut line = serde_json::to_string(message).expect("player line");
    line.push('\n');
    line
}

pub fn read_player(line: &str) -> Result<PlayerMessage, ProtocolError> {
    serde_json::from_str(trim_line(line)).map_err(|_| ProtocolError)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_say_line_round_trips() {
        let message = PlayerMessage::Say {
            from: "player:rowan".to_string(),
            address: "table".to_string(),
            text: "hello".to_string(),
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
        assert!(read_player(r#"{"kind":"check","outcome":"success","faces":[1]}"#).is_err());
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
}
