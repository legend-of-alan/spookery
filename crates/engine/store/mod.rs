//! Save, journal, session tape, pack lock, and the rules version stamped on saves and rounds.

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
pub use tape::{emit_tape_record, TapeRecord};

/// Rules version stamped on every save and every round.
/// A different version is refused. Only `spook migrate` reads a save from rules 11.
pub const RULES_REV: u64 = 21;

pub(crate) fn require_rules_rev(rev: u64) -> Result<(), Reason> {
    if rev == RULES_REV {
        Ok(())
    } else {
        Err(Reason::RulesRevMismatch)
    }
}
