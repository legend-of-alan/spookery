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
    RulesRev,
    UnknownMove,
    UnscopedPerceive,
    CommitConflict,
    HandEdit,
    Schema,
    NotInSpec,
    SessionFull,
    SeatTaken,
    MissingItem,
    LineTooLong,
    CantAfford,
    NotAWeapon,
    NoExit,
    Busy,
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
            Reason::RulesRev => "RULES_REV",
            Reason::UnknownMove => "UNKNOWN_MOVE",
            Reason::UnscopedPerceive => "UNSCOPED_PERCEIVE",
            Reason::CommitConflict => "COMMIT_CONFLICT",
            Reason::HandEdit => "HAND_EDIT",
            Reason::Schema => "SCHEMA",
            Reason::NotInSpec => "NOT_IN_SPEC",
            Reason::SessionFull => "SESSION_FULL",
            Reason::SeatTaken => "SEAT_TAKEN",
            Reason::MissingItem => "MISSING_ITEM",
            Reason::LineTooLong => "LINE_TOO_LONG",
            Reason::CantAfford => "CANT_AFFORD",
            Reason::NotAWeapon => "NOT_A_WEAPON",
            Reason::NoExit => "NO_EXIT",
            Reason::Busy => "BUSY",
        }
    }

    pub(crate) fn from_token(token: &str) -> Option<Self> {
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
            "RULES_REV" => Reason::RulesRev,
            "UNKNOWN_MOVE" => Reason::UnknownMove,
            "UNSCOPED_PERCEIVE" => Reason::UnscopedPerceive,
            "COMMIT_CONFLICT" => Reason::CommitConflict,
            "HAND_EDIT" => Reason::HandEdit,
            "SCHEMA" => Reason::Schema,
            "NOT_IN_SPEC" => Reason::NotInSpec,
            "SESSION_FULL" => Reason::SessionFull,
            "SEAT_TAKEN" => Reason::SeatTaken,
            "MISSING_ITEM" => Reason::MissingItem,
            "LINE_TOO_LONG" => Reason::LineTooLong,
            "CANT_AFFORD" => Reason::CantAfford,
            "NOT_A_WEAPON" => Reason::NotAWeapon,
            "NO_EXIT" => Reason::NoExit,
            "BUSY" => Reason::Busy,
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
        if let Reason::RulesRev = self {
            return f.write_str("RULES_REV: start a fresh save");
        }
        f.write_str(self.token())
    }
}

impl std::error::Error for Reason {}

#[cfg(test)]
mod tests {
    use super::Reason;

    const ALL: [Reason; 28] = [
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
        Reason::RulesRev,
        Reason::UnknownMove,
        Reason::UnscopedPerceive,
        Reason::CommitConflict,
        Reason::HandEdit,
        Reason::Schema,
        Reason::NotInSpec,
        Reason::SessionFull,
        Reason::SeatTaken,
        Reason::MissingItem,
        Reason::LineTooLong,
        Reason::CantAfford,
        Reason::NotAWeapon,
        Reason::NoExit,
        Reason::Busy,
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
            | Reason::RulesRev
            | Reason::UnknownMove
            | Reason::UnscopedPerceive
            | Reason::CommitConflict
            | Reason::HandEdit
            | Reason::Schema
            | Reason::NotInSpec
            | Reason::SessionFull
            | Reason::SeatTaken
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
            assert!(listed(reason));
            assert_eq!(Reason::from_token(reason.token()), Some(reason));
        }
        assert_eq!(Reason::from_token("SEAT_TAKEN"), Some(Reason::SeatTaken));
        assert_eq!(Reason::from_token("seat_taken"), None);
    }
}
