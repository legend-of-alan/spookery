//! The round crate has no cucumber harness. This is the scenario:
//!
//! ```gherkin
//! Feature: Replay matches the played save
//!
//!   Scenario: Random play replays to the same save
//!     Given the docks pack
//!     And a successful grit check adds one lamp oil
//!     When random legal player moves and habit rounds are played
//!     Then replaying the journal yields the state hash stored on the last record
//! ```
//!
//! The docks check has an empty effect list, so the roll is not in the save.
//! The oil is what makes skipping `resolve_check` on replay change `now.yaml`.
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use proptest::test_runner::{TestCaseError, TestRunner};

use super::support::{arb_rounds, assert_replay_matches, play_rounds, runner_config, TempPack};
use crate::testing::copy_docks;

#[test]
fn random_play_replays_to_the_same_save() {
    let mut runner = TestRunner::new(runner_config());
    runner
        .run(&arb_rounds(), |rounds| {
            let pack = TempPack(copy_docks("prop-replay"));
            play_rounds(&pack.0, &rounds).map_err(TestCaseError::fail)?;
            assert_replay_matches(&pack.0)
        })
        .unwrap_or_else(|err| panic!("replay property failed: {err}"));
}
