//! The round crate has no cucumber harness. This is the scenario:
//!
//! ```gherkin
//! Feature: Replay matches the played save
//!
//!   Scenario: Random play replays to the same save
//!     Given the docks pack
//!     And a successful grit check adds one lamp oil
//!     When random legal player moves and habit rounds are played
//!     Then replaying the journal from the snapshot yields a now.yaml byte-identical to the live save
//! ```
//!
//! The docks check has an empty effect list, so the roll is not in the save.
//! The oil is what makes skipping `resolve_check` on replay change `now.yaml`.
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use std::fs;

use proptest::test_runner::{TestCaseError, TestRunner};

use crate::from_journal;
use crate::prop_play::{configure_runner, copy_docks, pick_rounds, play_rounds, TempPack};

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
    let live = fs::read(pack.join("state/save.yaml")).map_err(|err| fail_msg(err.to_string()))?;
    let report = from_journal(pack, None, None).map_err(|err| fail_msg(err.to_string()))?;
    if report.now_yaml == live {
        return Ok(());
    }
    let live_text = String::from_utf8_lossy(&live);
    let replayed = String::from_utf8_lossy(&report.now_yaml);
    Err(fail_msg(first_diff(&live_text, &replayed)))
}

fn first_diff(live: &str, replayed: &str) -> String {
    for (index, (left, right)) in live.lines().zip(replayed.lines()).enumerate() {
        if left != right {
            return format!(
                "save differed at line {}: live {left:?} replayed {right:?}",
                index + 1
            );
        }
    }
    format!(
        "save differed in length: live {} bytes, replayed {} bytes",
        live.len(),
        replayed.len()
    )
}

fn fail_msg(message: String) -> TestCaseError {
    TestCaseError::fail(message)
}
