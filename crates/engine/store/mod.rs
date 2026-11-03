//! Save, journal, and the rules version stamped on both.

use crate::Reason;

pub(crate) mod commit;
pub(crate) mod durability;
pub(crate) mod journal;
pub(crate) mod lock;
pub(crate) mod records;
pub(crate) mod recovery;
pub(crate) mod save;
pub(crate) mod tape;

pub use lock::{lock_pack, PackGuard};
pub use save::{emit_save, parse_save};
pub use tape::{emit_tape_record, TapeRecord};

/// Rules version stamped on every save and every round.
/// A different version is refused. There is no migration.
pub const RULES_REV: u64 = 11;

pub(crate) fn require_rules_rev(rev: u64) -> Result<(), Reason> {
    if rev == RULES_REV {
        Ok(())
    } else {
        Err(Reason::RulesRevMismatch)
    }
}
