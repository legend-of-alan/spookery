//! Failure tokens.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Pinned failure tokens. The journal stores the same token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
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

    const ALL: [Reason; 29] = [
        Reason::IllegalClass,
        Reason::IllegalKnowledge,
        Reason::OutOfRange,
        Reason::Budget,
        Reason::PausedClock,
        Reason::UnknownKey,
        Reason::ViewerMismatch,
        Reason::KernelOnPlayer,
        Reason::Ineligible,
        Reason::ActorMismatch,
        Reason::TableEmpty,
        Reason::PackRevMismatch,
        Reason::RulesRev,
        Reason::UnknownMove,
        Reason::UnscopedPerceive,
        Reason::CommitConflict,
        Reason::HandEdit,
        Reason::TriggerLeak,
        Reason::Schema,
        Reason::NotInSpec,
        Reason::SessionFull,
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
            | Reason::ActorMismatch
            | Reason::TableEmpty
            | Reason::PackRevMismatch
            | Reason::RulesRev
            | Reason::UnknownMove
            | Reason::UnscopedPerceive
            | Reason::CommitConflict
            | Reason::HandEdit
            | Reason::TriggerLeak
            | Reason::Schema
            | Reason::NotInSpec
            | Reason::SessionFull
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
            assert!(listed(reason));
            assert!(!reason.token().is_empty());
            let stored = serde_json::to_string(&reason).unwrap();
            assert_eq!(stored, format!("\"{}\"", reason.token()));
            assert_eq!(serde_json::from_str::<Reason>(&stored).unwrap(), reason);
        }
    }
}
