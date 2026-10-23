//! Play one round and replay the journal.

pub(crate) mod play;
pub(crate) mod replay;
pub(crate) mod stage;
pub(crate) mod state;

pub(crate) use play::{advance_while_busy, catch_up, play};
pub use play::{migrate_pack, play_pack_once, RoundCause, RoundOutput};
pub(crate) use replay::resume;
pub use replay::{
    from_journal, verify_pack, verify_quick, ReplayError, ReplayReport, VerifyError, VerifyReport,
};
pub use stage::advance::plan_catchup;

#[cfg(test)]
mod prop;
