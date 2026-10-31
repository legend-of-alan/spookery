//! Player messages, GM messages, and the check band, refusal and speaker they share.

mod gm;
mod player;

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A check band on the player wire, the GM wire, and `commit_round` checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Success,
    Partial,
    Miss,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Miss => "miss",
        }
    }
}

pub use gm::{
    parse_args, CastKind, CloseArgs, CloseBody, CommitArgs, CommitBody, GmEvent, IntentCall,
    MenuRow, PackSummary, PerceiveArgs, Queued, QueuedMove, Roll, SayArgs, SayEvent, StatusArgs,
    StatusPause, StatusPresence, TableStatus,
};
pub use player::{
    exit_phrase, hello_line, parse_hello, ClockView, ExitView, ItemStack, KnownFact, MeterView,
    NoteView, PlayerFrame, View,
};

/// The game and door addresses `spook run` prints on stdout once both ports are bound.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename = "listening")]
pub struct Addrs {
    pub game: String,
    pub door: String,
}

/// The agent door for a fixed game port: the next port, on the same address.
pub fn default_door(game: std::net::SocketAddr) -> Option<std::net::SocketAddr> {
    let port = game.port().checked_add(1)?;
    Some(std::net::SocketAddr::new(game.ip(), port))
}

/// The perceive body budget in characters: the `perceive` default, the engine's trim
/// and the prompt's cast cut.
pub const PERCEIVE_BODY_CHARS: usize = 800;

/// Every refusal code. The `code` of a player `refused` line, a GM tool refusal and the MCP
/// error's `data`, and the token the journal stores on a round line (the engine's
/// `store::records::Refusal`).
/// Strict and `Copy` on purpose: `spook verify` must fail on a code it does not know.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Reason {
    UnknownEffect,
    IllegalKnowledge,
    OutOfRange,
    Budget,
    PausedClock,
    UnknownId,
    ViewerMismatch,
    EngineOnPlayer,
    Ineligible,
    ActorMismatch,
    TableEmpty,
    PackRevMismatch,
    RulesRevMismatch,
    UnknownMove,
    UnscopedPerceive,
    CommitConflict,
    HandEdit,
    Schema,
    NotInSpec,
    TableFull,
    SeatTaken,
    ChoosePc,
    MissingItem,
    LineTooLong,
    CantAfford,
    NotAWeapon,
    NoExit,
    Busy,
    TriggerLeak,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::UnknownEffect => "UNKNOWN_EFFECT",
            Reason::IllegalKnowledge => "ILLEGAL_KNOWLEDGE",
            Reason::OutOfRange => "OUT_OF_RANGE",
            Reason::Budget => "BUDGET",
            Reason::PausedClock => "PAUSED_CLOCK",
            Reason::UnknownId => "UNKNOWN_ID",
            Reason::ViewerMismatch => "VIEWER_MISMATCH",
            Reason::EngineOnPlayer => "ENGINE_ON_PLAYER",
            Reason::Ineligible => "INELIGIBLE",
            Reason::ActorMismatch => "ACTOR_MISMATCH",
            Reason::TableEmpty => "TABLE_EMPTY",
            Reason::PackRevMismatch => "PACK_REV_MISMATCH",
            Reason::RulesRevMismatch => "RULES_REV_MISMATCH",
            Reason::UnknownMove => "UNKNOWN_MOVE",
            Reason::UnscopedPerceive => "UNSCOPED_PERCEIVE",
            Reason::CommitConflict => "COMMIT_CONFLICT",
            Reason::HandEdit => "HAND_EDIT",
            Reason::Schema => "SCHEMA",
            Reason::NotInSpec => "NOT_IN_SPEC",
            Reason::TableFull => "TABLE_FULL",
            Reason::SeatTaken => "SEAT_TAKEN",
            Reason::ChoosePc => "CHOOSE_PC",
            Reason::MissingItem => "MISSING_ITEM",
            Reason::LineTooLong => "LINE_TOO_LONG",
            Reason::CantAfford => "CANT_AFFORD",
            Reason::NotAWeapon => "NOT_A_WEAPON",
            Reason::NoExit => "NO_EXIT",
            Reason::Busy => "BUSY",
            Reason::TriggerLeak => "TRIGGER_LEAK",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Reason::RulesRevMismatch = self {
            return f.write_str("RULES_REV_MISMATCH: start a fresh save");
        }
        f.write_str(self.as_str())
    }
}

impl std::error::Error for Reason {}

/// The code as a client reads it off the wire: a `Reason` this build knows, or the
/// text of one it does not. Serializes as the bare string either way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Code {
    Known(Reason),
    Other(String),
}

impl Code {
    /// The token as it is written on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Code::Known(reason) => reason.as_str(),
            Code::Other(code) => code,
        }
    }
}

impl From<String> for Code {
    /// A `Reason` this build knows, read through its serde token, or the text as it came.
    fn from(code: String) -> Self {
        match serde_json::from_value(serde_json::Value::String(code.clone())) {
            Ok(reason) => Code::Known(reason),
            Err(_) => Code::Other(code),
        }
    }
}

/// Sent from the table to a player client as the `refused` line's body, and to a GM
/// client as a tool refusal (`{"code"}`) and as the MCP initialize error's `data`.
/// Display is that error's message, the sentence with the code in brackets,
/// `The table is full. [TABLE_FULL]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub code: Code,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Detail>,
}

impl From<Reason> for Refusal {
    fn from(code: Reason) -> Self {
        Self {
            code: Code::Known(code),
            detail: None,
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match &self.code {
            Code::Known(reason) => *reason,
            Code::Other(code) => return write!(f, "The table refused that. [{code}]"),
        };
        let code = reason.as_str();
        let named = match &self.detail {
            Some(Detail::One(text)) if !text.is_empty() => Some(text),
            _ => None,
        };
        let sentence = match reason {
            Reason::UnknownEffect => "That action is not allowed.",
            Reason::IllegalKnowledge => "You cannot know that.",
            Reason::OutOfRange => "That is out of range.",
            Reason::Budget => "That does not fit.",
            Reason::PausedClock => "That clock is paused.",
            Reason::UnknownId => "That is not in the world.",
            Reason::ViewerMismatch => "That is not your view.",
            Reason::EngineOnPlayer => "The table cannot do that for you.",
            Reason::Ineligible => "You cannot do that right now.",
            Reason::ActorMismatch => "That action is not yours.",
            Reason::TableEmpty => "Nobody is seated.",
            Reason::PackRevMismatch => "The save does not match this pack.",
            Reason::UnknownMove => "That is not a move.",
            Reason::UnscopedPerceive => "You cannot see that.",
            Reason::CommitConflict => "The table was already writing. Try again.",
            Reason::HandEdit => "The save was edited by hand.",
            Reason::Schema => "That line is not valid.",
            Reason::NotInSpec => "The table does not understand that. Type help.",
            Reason::TableFull => "The table is full.",
            Reason::SeatTaken => "That character is already taken.",
            Reason::MissingItem => "You do not have that.",
            Reason::NotAWeapon => "That is not a weapon.",
            Reason::LineTooLong => "That line is too long.",
            Reason::ChoosePc => match &self.detail {
                Some(Detail::Many(pcs)) if !pcs.is_empty() => {
                    return write!(f, "Choose a character: {}. [{code}]", pcs.join(", "));
                }
                _ => "Choose a character.",
            },
            Reason::NoExit => match named {
                Some(text) => return write!(f, "{text} [{code}]"),
                None => "There is no exit that way.",
            },
            Reason::Busy => "You are busy.",
            Reason::CantAfford => match named {
                Some(what) => return write!(f, "You cannot afford {what}. [{code}]"),
                None => "You cannot afford that.",
            },
            Reason::RulesRevMismatch | Reason::TriggerLeak => "The table refused that.",
        };
        write!(f, "{sentence} [{code}]")
    }
}

/// A refusal's `detail`: one id or locked text, or the free PCs on `CHOOSE_PC`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Detail {
    One(String),
    Many(Vec<String>),
}

/// Who spoke: the `from` of a say on the player wire, the GM wire and the tape, and the
/// player hello line a player client sends the table. Written `gm`, `player:<id>` or
/// `npc:<id>`; `FromStr` refuses an empty or whitespace id and any other prefix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Speaker {
    Gm,
    Player(String),
    Npc(String),
}

impl fmt::Display for Speaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Speaker::Gm => f.write_str("gm"),
            Speaker::Player(id) => write!(f, "player:{id}"),
            Speaker::Npc(id) => write!(f, "npc:{id}"),
        }
    }
}

impl FromStr for Speaker {
    type Err = Reason;

    fn from_str(text: &str) -> Result<Self, Reason> {
        if text == "gm" {
            return Ok(Speaker::Gm);
        }
        let (kind, id) = text.split_once(':').ok_or(Reason::Schema)?;
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return Err(Reason::Schema);
        }
        match kind {
            "player" => Ok(Speaker::Player(id.to_string())),
            "npc" => Ok(Speaker::Npc(id.to_string())),
            _ => Err(Reason::Schema),
        }
    }
}

impl TryFrom<String> for Speaker {
    type Error = Reason;

    fn try_from(text: String) -> Result<Self, Reason> {
        text.parse()
    }
}

impl From<Speaker> for String {
    fn from(speaker: Speaker) -> Self {
        speaker.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Reason; 29] = [
        Reason::UnknownEffect,
        Reason::IllegalKnowledge,
        Reason::OutOfRange,
        Reason::Budget,
        Reason::PausedClock,
        Reason::UnknownId,
        Reason::ViewerMismatch,
        Reason::EngineOnPlayer,
        Reason::Ineligible,
        Reason::ActorMismatch,
        Reason::TableEmpty,
        Reason::PackRevMismatch,
        Reason::RulesRevMismatch,
        Reason::UnknownMove,
        Reason::UnscopedPerceive,
        Reason::CommitConflict,
        Reason::HandEdit,
        Reason::TriggerLeak,
        Reason::Schema,
        Reason::NotInSpec,
        Reason::TableFull,
        Reason::SeatTaken,
        Reason::ChoosePc,
        Reason::MissingItem,
        Reason::LineTooLong,
        Reason::CantAfford,
        Reason::NotAWeapon,
        Reason::NoExit,
        Reason::Busy,
    ];

    /// A new variant fails to compile here until it is added to `ALL`.
    fn is_listed(reason: Reason) -> bool {
        match reason {
            Reason::UnknownEffect
            | Reason::IllegalKnowledge
            | Reason::OutOfRange
            | Reason::Budget
            | Reason::PausedClock
            | Reason::UnknownId
            | Reason::ViewerMismatch
            | Reason::EngineOnPlayer
            | Reason::Ineligible
            | Reason::ActorMismatch
            | Reason::TableEmpty
            | Reason::PackRevMismatch
            | Reason::RulesRevMismatch
            | Reason::UnknownMove
            | Reason::UnscopedPerceive
            | Reason::CommitConflict
            | Reason::HandEdit
            | Reason::TriggerLeak
            | Reason::Schema
            | Reason::NotInSpec
            | Reason::TableFull
            | Reason::SeatTaken
            | Reason::ChoosePc
            | Reason::MissingItem
            | Reason::LineTooLong
            | Reason::CantAfford
            | Reason::NotAWeapon
            | Reason::NoExit
            | Reason::Busy => ALL.contains(&reason),
        }
    }

    #[test]
    fn every_token_round_trips() {
        for reason in ALL {
            assert!(is_listed(reason));
            assert!(!reason.as_str().is_empty());
            let stored = serde_json::to_string(&reason).unwrap();
            assert_eq!(stored, format!("\"{}\"", reason.as_str()));
            assert_eq!(serde_json::from_str::<Reason>(&stored).unwrap(), reason);
            let code = serde_json::from_str::<Code>(&stored).unwrap();
            assert_eq!(code, Code::Known(reason));
        }
        assert!(serde_json::from_str::<Reason>("\"NEW_CODE\"").is_err());
    }

    #[test]
    fn a_refusal_is_a_sentence_with_the_code() {
        let line = |code: Reason| Refusal::from(code).to_string();
        assert_eq!(line(Reason::TableFull), "The table is full. [TABLE_FULL]");
        assert!(line(Reason::SeatTaken) == "That character is already taken. [SEAT_TAKEN]");
        assert!(line(Reason::LineTooLong) == "That line is too long. [LINE_TOO_LONG]");
        assert_eq!(
            line(Reason::RulesRevMismatch),
            "The table refused that. [RULES_REV_MISMATCH]"
        );
        let unknown = Refusal {
            code: Code::Other("NEW_CODE".into()),
            detail: None,
        };
        assert_eq!(unknown.to_string(), "The table refused that. [NEW_CODE]");
        assert_eq!(line(Reason::ChoosePc), "Choose a character. [CHOOSE_PC]");
        let detailed = |code: Reason, detail: Detail| {
            Refusal {
                code: Code::Known(code),
                detail: Some(detail),
            }
            .to_string()
        };
        assert_eq!(
            detailed(Reason::ChoosePc, Detail::Many(vec!["pc_rowan".into()])),
            "Choose a character: pc_rowan. [CHOOSE_PC]"
        );
        assert_eq!(
            detailed(Reason::CantAfford, Detail::One("g".into())),
            "You cannot afford g. [CANT_AFFORD]"
        );
        for raw in [
            r#"{"kind":"refused","code":"CANT_AFFORD","detail":"grit"}"#,
            r#"{"kind":"refused","code":"CHOOSE_PC","detail":["pc_rowan"]}"#,
            r#"{"kind":"refused","code":"SCHEMA"}"#,
            r#"{"kind":"refused","code":"NEW_CODE"}"#,
        ] {
            let msg = PlayerFrame::from_line(raw).unwrap();
            assert!(matches!(msg, PlayerFrame::Refused(_)));
            assert_eq!(serde_json::to_string(&msg).unwrap(), raw);
        }
    }

    #[test]
    fn a_fixed_game_port_dials_the_next_port() {
        let game = std::net::SocketAddr::from(([127, 0, 0, 1], 9737));
        let door = std::net::SocketAddr::from(([127, 0, 0, 1], 9738));
        assert_eq!(default_door(game), Some(door));
    }

    #[test]
    fn a_speaker_is_gm_or_a_prefixed_id() {
        for (text, speaker) in [
            ("gm", Speaker::Gm),
            ("player:pc_rowan", Speaker::Player("pc_rowan".into())),
            ("npc:mira", Speaker::Npc("mira".into())),
            ("player:a:b", Speaker::Player("a:b".into())),
        ] {
            assert_eq!(text.parse::<Speaker>(), Ok(speaker.clone()));
            assert_eq!(speaker.to_string(), text);
        }
        for text in [
            "",
            "somebody",
            "player:",
            "npc:",
            "player:a b",
            "pc:rowan",
            "gm ",
        ] {
            assert_eq!(text.parse::<Speaker>(), Err(Reason::Schema), "{text}");
        }
    }
}
