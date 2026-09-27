use std::fmt;

/// Pinned failure tokens. JSONL lowers `BUDGET` and `SKIPPED` at the log edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
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

impl Code {
    pub fn token(self) -> &'static str {
        match self {
            Code::IllegalClass => "ILLEGAL_CLASS",
            Code::IllegalKnowledge => "ILLEGAL_KNOWLEDGE",
            Code::OutOfRange => "OUT_OF_RANGE",
            Code::Budget => "BUDGET",
            Code::PausedClock => "PAUSED_CLOCK",
            Code::UnknownKey => "UNKNOWN_KEY",
            Code::NotOwner => "NOT_OWNER",
            Code::PackRevMismatch => "PACK_REV_MISMATCH",
            Code::UnknownMove => "UNKNOWN_MOVE",
            Code::LiveOnly => "LIVE_ONLY",
            Code::UnscopedPerceive => "UNSCOPED_PERCEIVE",
            Code::DeniedTool => "DENIED_TOOL",
            Code::CommitConflict => "COMMIT_CONFLICT",
            Code::HandEdit => "HAND_EDIT",
            Code::LlmDisabled => "LLM_DISABLED",
            Code::Schema => "SCHEMA",
            Code::NotInSpec => "NOT_IN_SPEC",
            Code::SessionFull => "SESSION_FULL",
            Code::Skipped => "SKIPPED",
        }
    }

    /// JSONL `rejected.reason`. `budget` and `skipped` are lowercase words.
    pub fn jsonl_reason(self) -> &'static str {
        match self {
            Code::Budget => "budget",
            Code::Skipped => "skipped",
            other => other.token(),
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

impl std::error::Error for Code {}
