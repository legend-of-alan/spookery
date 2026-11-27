//! Save, journal, session tape, pack lock, and the rules version stamped on saves and rounds.

use crate::Reason;

pub(crate) mod commit;
pub(crate) mod durability;
pub(crate) mod journal;
pub mod lock;
pub(crate) mod records;
pub(crate) mod recovery;
pub(crate) mod save;
pub mod tape;

pub use lock::PackGuard;

/// Rules version stamped on every save and every round.
/// A different version is refused.
#[cfg_attr(not(feature = "testing"), expect(unreachable_pub))]
pub const RULES_REV: u64 = 35;

pub(crate) fn check_rules_rev(rev: u64) -> Result<(), Reason> {
    if rev == RULES_REV {
        Ok(())
    } else {
        Err(Reason::RulesRevMismatch)
    }
}
