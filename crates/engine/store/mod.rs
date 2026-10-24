//! Save, journal, and the rules version stamped on both.

use crate::error::Reason;

pub(crate) mod codec;
pub(crate) mod commit;
pub(crate) mod durable;
pub(crate) mod journal;
pub(crate) mod lock;
pub(crate) mod records;
pub(crate) mod recover;
pub(crate) mod tape;

pub use codec::{emit_save, load_save};
pub use lock::{lock_serve, LockGuard};
pub use tape::{record_line, TapeRecord};

/// Rules version stamped on every save and every tick.
/// A different version is refused. There is no migration.
pub const RULES_REV: u64 = 7;

pub(crate) fn require_rules_rev(rev: u64) -> Result<(), Reason> {
    if rev == RULES_REV {
        Ok(())
    } else {
        Err(Reason::RulesRev)
    }
}
