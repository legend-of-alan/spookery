//! The player wire, the GM wire, and the check band they share.

mod gm_wire;
mod player_wire;

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

pub use gm_wire::{
    code_result, queued_ack, read_commit_args, read_intent_args, read_perceive_args, read_say_args,
    read_status_args, read_tool_result, released_result, write_check_event, write_queued_event,
    write_say_event, CallTarget, CommitArgs, CommitBody, EventBody, GmSay, IntentCall, OutgoingSay,
    PerceiveArgs, QueueRow, Roll, SessionStatus, StatusArgs, StatusPause, StatusPresence,
};
pub use player_wire::{
    read_player, read_player_hello, write_player, write_player_hello, PlayerMessage, ViewScene,
    WireClock, WireExit, WireFact, WireItem, WireMessage, WireMeter,
};

/// A line that is not one of the two message sets.
#[derive(Debug, PartialEq, Eq)]
pub struct Error;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("protocol")
    }
}

impl std::error::Error for Error {}
