//! Play one round and replay the journal.

pub(crate) mod dice;
pub(crate) mod log;
pub(crate) mod play;
pub(crate) mod replay;
pub(crate) mod stage;

pub(crate) use play::{play_catch_up, play_round, play_while_busy};
pub use play::{play_pack_once, RoundOutput};
pub use replay::{migrate_pack, verify_pack, verify_quick};

#[cfg(test)]
mod prop;
