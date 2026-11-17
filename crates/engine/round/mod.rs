//! Play one round and replay the journal.

pub(crate) mod dice;
pub(crate) mod ledger;
pub(crate) mod play;
pub(crate) mod replay;
pub(crate) mod stage;
pub(crate) mod verify;

pub(crate) use dice::Dice;
pub(crate) use ledger::Ledger;
pub(crate) use play::{fast_forward, play_catch_up, play_round};
pub use play::{play_pack_once, Output};
pub use replay::migrate_pack;
pub(crate) use stage::advance::Time;
pub use verify::{verify_pack, verify_quick};

#[cfg(test)]
mod prop;
