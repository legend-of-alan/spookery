//! Play one round and replay the journal.
//!
//! Public API
//!
//! No module is public. Callers use the functions re-exported below.
//!
//! Internal
//! - `play`: run a round, commit it, catch the table up, and migrate.
//! - `replay`: walk the journal and check that the save matches.
//! - `seed`: the round line seed, and the key that shuffles intents.
//! - `stage`: one round, in the order `play` runs them.
//! - `state`: one pass of the rules over one tick.
//! - `prop`: property tests for play and replay. Compiled only for `cargo test`.

mod play;
mod replay;
mod seed;
mod stage;
mod state;

pub use play::{
    advance_while_busy, catch_up, commit_moves, commit_moves_at, migrate_pack, play,
    play_pack_once, RoundCause, RoundOutput,
};
pub use replay::{
    from_journal, replay_states, resume, verify_pack, verify_parallel, verify_quick, ReplayError,
    ReplayReport, ReplayStep, VerifyError, VerifyReport,
};
pub use stage::advance::plan_catchup;

#[cfg(test)]
mod prop;
