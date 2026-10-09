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
    ViewerMismatch,
    KernelOnPlayer,
    Ineligible,
    NoProfile,
    ActorMismatch,
    TableEmpty,
    PackRevMismatch,
    UnknownMove,
    UnscopedPerceive,
    CommitConflict,
    HandEdit,
    Schema,
    NotInSpec,
    SessionFull,
    MissingItem,
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
            Reason::ViewerMismatch => "VIEWER_MISMATCH",
            Reason::KernelOnPlayer => "KERNEL_ON_PLAYER",
            Reason::Ineligible => "INELIGIBLE",
            Reason::NoProfile => "NO_PROFILE",
            Reason::ActorMismatch => "ACTOR_MISMATCH",
            Reason::TableEmpty => "TABLE_EMPTY",
            Reason::PackRevMismatch => "PACK_REV_MISMATCH",
            Reason::UnknownMove => "UNKNOWN_MOVE",
            Reason::UnscopedPerceive => "UNSCOPED_PERCEIVE",
            Reason::CommitConflict => "COMMIT_CONFLICT",
            Reason::HandEdit => "HAND_EDIT",
            Reason::Schema => "SCHEMA",
            Reason::NotInSpec => "NOT_IN_SPEC",
            Reason::SessionFull => "SESSION_FULL",
            Reason::MissingItem => "MISSING_ITEM",
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
