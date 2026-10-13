mod gm;
mod player;

use serde::{Deserialize, Serialize};

/// A check band on the player wire, the GM wire, and `commit_turn` checks.
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
    code_result, queued_ack, read_commit_args, read_frame, read_intent_args, read_perceive_args,
    read_say_args, read_status_args, read_tool_call, read_tool_result, set_checks,
    write_check_event, write_queued_event, write_say_event, write_tool_call, CallTarget,
    CommitArgs, CommitBody, Frame, GmSay, IntentCall, PerceiveArgs, QueueRow, Roll, SayArgs,
    SessionStatus, StatusArgs, StatusPause, StatusPresence, ToolCall, GM_HELLO, INITIALIZED_LINE,
    INITIALIZE_LINE,
};
pub use player::{
    read_player, read_player_hello, write_player, write_player_hello, FactNote, PlayerMessage,
    WireClock, WireFact, WireStock,
};

/// A line that is not one of the two message sets.
#[derive(Debug, PartialEq, Eq)]
pub struct ProtocolError;

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("protocol")
    }
}

impl std::error::Error for ProtocolError {}
