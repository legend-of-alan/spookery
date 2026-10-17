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
use spookery_kernel::api::journal;

use super::play::{configure_runner, copy_docks, pick_rounds, play_rounds, TempPack};
use crate::from_journal;

#[test]
fn random_play_replays_to_the_same_save() {
    let mut runner = TestRunner::new(configure_runner());
    runner
        .run(&pick_rounds(), |rounds| {
            let pack = TempPack(copy_docks("prop-replay"));
            play_rounds(&pack.0, &rounds).map_err(fail_msg)?;
            same_save(&pack.0)
        })
        .unwrap_or_else(|err| panic!("replay property failed: {err}"));
}

fn same_save(pack: &std::path::Path) -> Result<(), TestCaseError> {
    let report = from_journal(pack, None, None).map_err(|err| fail_msg(err.to_string()))?;
    let entries = journal::read_entries(pack)
        .map_err(|fault| fail_msg(format!("{}: {}", fault.file, fault.detail)))?;
    let Some(last) = entries.iter().rev().find(|entry| entry.changes_state()) else {
        return Ok(());
    };
    let replayed = journal::hash_now(&report.now_yaml);
    if replayed == last.now_hash() {
        return Ok(());
    }
    Err(fail_msg(format!(
        "state hash differed: record {} replayed {replayed}",
        last.now_hash()
    )))
}

fn fail_msg(message: String) -> TestCaseError {
    TestCaseError::fail(message)
}
