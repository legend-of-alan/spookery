//! Player messages, GM messages, and the check band they share.

mod gm;
mod player;

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
    code_result, parse_commit_args, parse_intent_args, parse_perceive_args, parse_say_args,
    parse_status_args, parse_tool_result, queued_result, released_result, CastKind, CommitArgs,
    CommitBody, EventBody, IntentCall, MenuRow, PackSummary, PerceiveArgs, QueuedMove, Roll,
    SayArgs, SayEvent, StatusArgs, StatusPause, StatusPresence, TableStatus,
};
pub use player::{
    emit_player, emit_player_hello, exit_phrase, parse_player, parse_player_hello, ClockView,
    ExitView, ItemStack, KnownFact, MeterView, Missing, NoteView, PlayerFrame, ViewScene,
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

pub fn refusal(code: &str) -> String {
    named_refusal(code, None)
}

pub fn named_refusal(code: &str, missing: Option<&Missing>) -> String {
    let named = missing
        .and_then(Missing::as_str)
        .filter(|text| !text.is_empty());
    let sentence = match code {
        "UNKNOWN_EFFECT" => "That action is not allowed.",
        "ILLEGAL_KNOWLEDGE" => "You cannot know that.",
        "OUT_OF_RANGE" => "That is out of range.",
        "BUDGET" => "That does not fit.",
        "PAUSED_CLOCK" => "That clock is paused.",
        "UNKNOWN_ID" => "That is not in the world.",
        "VIEWER_MISMATCH" => "That is not your view.",
        "ENGINE_ON_PLAYER" => "The table cannot do that for you.",
        "INELIGIBLE" => "You cannot do that right now.",
        "ACTOR_MISMATCH" => "That action is not yours.",
        "TABLE_EMPTY" => "Nobody is seated.",
        "PACK_REV_MISMATCH" => "The save does not match this pack.",
        "UNKNOWN_MOVE" => "That is not a move.",
        "UNSCOPED_PERCEIVE" => "You cannot see that.",
        "COMMIT_CONFLICT" => "The table was already writing. Try again.",
        "HAND_EDIT" => "The save was edited by hand.",
        "SCHEMA" => "That line is not valid.",
        "NOT_IN_SPEC" => "The table does not understand that. Type help.",
        "TABLE_FULL" => "The table is full.",
        "SEAT_TAKEN" => "That character is already taken.",
        "MISSING_ITEM" => "You do not have that.",
        "NOT_A_WEAPON" => "That is not a weapon.",
        "LINE_TOO_LONG" => "That line is too long.",
        "CHOOSE_PC" => match missing {
            Some(Missing::Many(pcs)) if !pcs.is_empty() => {
                return format!("Choose a character: {}. [{code}]", pcs.join(", "));
            }
            _ => "Choose a character.",
        },
        "NO_EXIT" => match named {
            Some(text) => return format!("{text} [{code}]"),
            None => "There is no exit that way.",
        },
        "BUSY" => "You are busy.",
        "CANT_AFFORD" => match named {
            Some(what) => return format!("You cannot afford {what}. [{code}]"),
            None => "You cannot afford that.",
        },
        _ => "The table refused that.",
    };
    format!("{sentence} [{code}]")
}

/// A line that is not one of the two message sets.
#[derive(Debug, PartialEq, Eq)]
pub struct Error;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("protocol")
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_a_sentence_with_the_code() {
        assert_eq!(refusal("TABLE_FULL"), "The table is full. [TABLE_FULL]");
        assert!(refusal("SEAT_TAKEN") == "That character is already taken. [SEAT_TAKEN]");
        assert!(refusal("LINE_TOO_LONG") == "That line is too long. [LINE_TOO_LONG]");
        assert_eq!(refusal("NEW_CODE"), "The table refused that. [NEW_CODE]");
        assert_eq!(refusal("CHOOSE_PC"), "Choose a character. [CHOOSE_PC]");
        assert_eq!(
            named_refusal("CHOOSE_PC", Some(&Missing::Many(vec!["pc_rowan".into()]))),
            "Choose a character: pc_rowan. [CHOOSE_PC]"
        );
        assert_eq!(
            named_refusal("CANT_AFFORD", Some(&Missing::One("g".into()))),
            "You cannot afford g. [CANT_AFFORD]"
        );
        for raw in [
            r#"{"kind":"refused","code":"CANT_AFFORD","detail":"grit"}"#,
            r#"{"kind":"refused","code":"CHOOSE_PC","detail":["pc_rowan"]}"#,
            r#"{"kind":"refused","code":"SCHEMA"}"#,
        ] {
            let msg = parse_player(raw).unwrap();
            assert!(matches!(msg, PlayerFrame::Refused { .. }));
            assert_eq!(serde_json::to_string(&msg).unwrap(), raw);
        }
    }
}
