//! Save, journal, and the rules version stamped on both.

use crate::error::Reason;

pub mod codec;
pub(crate) mod durable;
pub(crate) mod recover;
pub mod talk_log;

/// Rules version stamped on every save and every tick.
/// A different version is refused. There is no migration.
pub const RULES_REV: u64 = 4;

pub fn require_rules_rev(rev: u64) -> Result<(), Reason> {
    if rev == RULES_REV {
        Ok(())
    } else {
        Err(Reason::RulesRev)
    }
}
