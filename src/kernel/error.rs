use std::fmt;

/// Pinned failure tokens. JSONL lowers `BUDGET` and `SKIPPED` at the log edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    IllegalClass,
    IllegalKnowledge,
    OutOfRange,
    Budget,
    PausedClock,
    UnknownKey,
    NotOwner,
    PackRevMismatch,
    UnknownMove,
    LiveOnly,
    UnscopedPerceive,
    DeniedTool,
    CommitConflict,
    HandEdit,
    LlmDisabled,
    Schema,
    NotInSpec,
    SessionFull,
    Skipped,
}

impl Reason {
    pub fn token(self) -> &'static str {
        match self {
            Reason::IllegalClass => "ILLEGAL_CLASS",
            Reason::IllegalKnowledge => "ILLEGAL_KNOWLEDGE",
            Reason::OutOfRange => "OUT_OF_RANGE",
            Reason::Budget => "BUDGET",
            Reason::PausedClock => "PAUSED_CLOCK",
            Reason::UnknownKey => "UNKNOWN_KEY",
            Reason::NotOwner => "NOT_OWNER",
            Reason::PackRevMismatch => "PACK_REV_MISMATCH",
            Reason::UnknownMove => "UNKNOWN_MOVE",
            Reason::LiveOnly => "LIVE_ONLY",
            Reason::UnscopedPerceive => "UNSCOPED_PERCEIVE",
            Reason::DeniedTool => "DENIED_TOOL",
            Reason::CommitConflict => "COMMIT_CONFLICT",
            Reason::HandEdit => "HAND_EDIT",
            Reason::LlmDisabled => "LLM_DISABLED",
            Reason::Schema => "SCHEMA",
            Reason::NotInSpec => "NOT_IN_SPEC",
            Reason::SessionFull => "SESSION_FULL",
            Reason::Skipped => "SKIPPED",
        }
    }

    /// JSONL `rejected.reason`. `budget` and `skipped` are lowercase words.
    pub fn jsonl_reason(self) -> &'static str {
        match self {
            Reason::Budget => "budget",
            Reason::Skipped => "skipped",
            other => other.token(),
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

impl std::error::Error for Reason {}
