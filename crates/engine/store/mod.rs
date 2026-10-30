//! Save, journal, and the rules version stamped on both.

use crate::error::Reason;

pub(crate) mod codec;
pub(crate) mod commit;
pub(crate) mod durability;
pub(crate) mod journal;
pub(crate) mod lock;
pub(crate) mod records;
pub(crate) mod recovery;
pub(crate) mod tape;

pub use codec::{emit_save, parse_save};
pub use lock::{lock_pack, PackGuard};
pub use tape::{emit_tape_record, TapeRecord};

/// Rules version stamped on every save and every round.
/// A different version is refused. There is no migration.
pub const RULES_REV: u64 = 9;

pub(crate) fn require_rules_rev(rev: u64) -> Result<(), Reason> {
    if rev == RULES_REV {
        Ok(())
    } else {
        Err(Reason::RulesRevMismatch)
    }
}
