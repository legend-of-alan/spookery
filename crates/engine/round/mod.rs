//! Play one round and replay the journal.

pub(crate) mod play;
pub(crate) mod replay;
pub(crate) mod stage;
pub(crate) mod state;

pub(crate) use play::{advance_while_busy, catch_up, play};
pub use play::{migrate_pack, play_pack_once, RoundCause, RoundOutput};
#[allow(unused_imports)]
pub(crate) use replay::{from_journal, resume, ReplayError, ReplayReport};
pub use replay::{verify_pack, verify_quick};
pub use stage::advance::plan_catchup;

#[cfg(test)]
mod prop;
