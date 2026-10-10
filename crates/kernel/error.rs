use std::fmt;

/// Pinned failure tokens. JSONL lowers `BUDGET` at the log edge.
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
    SeatTaken,
    MissingItem,
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
            Reason::SeatTaken => "SEAT_TAKEN",
            Reason::MissingItem => "MISSING_ITEM",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "ILLEGAL_CLASS" => Reason::IllegalClass,
            "ILLEGAL_KNOWLEDGE" => Reason::IllegalKnowledge,
            "OUT_OF_RANGE" => Reason::OutOfRange,
            "BUDGET" => Reason::Budget,
            "PAUSED_CLOCK" => Reason::PausedClock,
            "UNKNOWN_KEY" => Reason::UnknownKey,
            "VIEWER_MISMATCH" => Reason::ViewerMismatch,
            "KERNEL_ON_PLAYER" => Reason::KernelOnPlayer,
            "INELIGIBLE" => Reason::Ineligible,
            "NO_PROFILE" => Reason::NoProfile,
            "ACTOR_MISMATCH" => Reason::ActorMismatch,
            "TABLE_EMPTY" => Reason::TableEmpty,
            "PACK_REV_MISMATCH" => Reason::PackRevMismatch,
            "UNKNOWN_MOVE" => Reason::UnknownMove,
            "UNSCOPED_PERCEIVE" => Reason::UnscopedPerceive,
            "COMMIT_CONFLICT" => Reason::CommitConflict,
            "HAND_EDIT" => Reason::HandEdit,
            "SCHEMA" => Reason::Schema,
            "NOT_IN_SPEC" => Reason::NotInSpec,
            "SESSION_FULL" => Reason::SessionFull,
            "SEAT_TAKEN" => Reason::SeatTaken,
            "MISSING_ITEM" => Reason::MissingItem,
            _ => return None,
        })
    }

    /// JSONL `rejected.reason`. `budget` is the lowercase word.
    pub fn jsonl_reason(self) -> &'static str {
        match self {
            Reason::Budget => "budget",
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

#[cfg(test)]
mod tests {
    use super::Reason;

    const ALL: [Reason; 22] = [
        Reason::IllegalClass,
        Reason::IllegalKnowledge,
        Reason::OutOfRange,
        Reason::Budget,
        Reason::PausedClock,
        Reason::UnknownKey,
        Reason::ViewerMismatch,
        Reason::KernelOnPlayer,
        Reason::Ineligible,
        Reason::NoProfile,
        Reason::ActorMismatch,
        Reason::TableEmpty,
        Reason::PackRevMismatch,
        Reason::UnknownMove,
        Reason::UnscopedPerceive,
        Reason::CommitConflict,
        Reason::HandEdit,
        Reason::Schema,
        Reason::NotInSpec,
        Reason::SessionFull,
        Reason::SeatTaken,
        Reason::MissingItem,
    ];

    /// A new variant fails to compile here until it is added to `ALL`.
    fn listed(reason: Reason) -> bool {
        match reason {
            Reason::IllegalClass
            | Reason::IllegalKnowledge
            | Reason::OutOfRange
            | Reason::Budget
            | Reason::PausedClock
            | Reason::UnknownKey
            | Reason::ViewerMismatch
            | Reason::KernelOnPlayer
            | Reason::Ineligible
            | Reason::NoProfile
            | Reason::ActorMismatch
            | Reason::TableEmpty
            | Reason::PackRevMismatch
            | Reason::UnknownMove
            | Reason::UnscopedPerceive
            | Reason::CommitConflict
            | Reason::HandEdit
            | Reason::Schema
            | Reason::NotInSpec
            | Reason::SessionFull
            | Reason::SeatTaken
            | Reason::MissingItem => ALL.contains(&reason),
        }
    }

    #[test]
    fn every_token_round_trips() {
        for reason in ALL {
            assert!(listed(reason));
            assert_eq!(Reason::from_token(reason.token()), Some(reason));
        }
        assert_eq!(Reason::from_token("SEAT_TAKEN"), Some(Reason::SeatTaken));
        assert_eq!(Reason::from_token("seat_taken"), None);
    }
}
