//! The round crate has no cucumber harness. This is the scenario:
//!
//! ```gherkin
//! Feature: Re-run matches the journal
//!
//!   Scenario: Running a round's effects again matches the logged effects
//!     Given the docks pack
//!     And a successful exert check adds one lamp oil
//!     When random legal player verbs and habit rounds are played
//!     Then running each round again yields the effects written in the journal
//! ```
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use std::fs;
use std::path::Path;

use crate::session;
use crate::store::journal;
use crate::store::records;
use crate::store::save;
use proptest::test_runner::{TestCaseError, TestRunner};

use super::support::{arb_rounds, play_rounds, runner_config, TempPack};
use crate::ecs::Presence;
use crate::replay;
use crate::round;
use crate::testing::{copy_dir, copy_pack, fresh_dir};

#[test]
fn running_a_round_again_matches_the_logged_effects() {
    let mut runner = TestRunner::new(runner_config());
    runner
        .run(&arb_rounds(), |rounds| {
            let played = TempPack(copy_pack("docks", "prop-rerun"));
            play_rounds(&played.0, &rounds).map_err(TestCaseError::fail)?;
            replay_each_round(&played.0)
        })
        .unwrap_or_else(|err| panic!("re-run property failed: {err}"));
}

fn replay_each_round(played: &Path) -> Result<(), TestCaseError> {
    let entries = journal::read_verified(played)
        .map_err(|fault| TestCaseError::fail(format!("{}: {}", fault.file, fault.detail)))?;
    let round_records: Vec<_> = entries
        .into_iter()
        .filter_map(|entry| match entry {
            records::Record::Round(record) => Some(record),
            _ => None,
        })
        .collect();
    if round_records.is_empty() {
        return Err(TestCaseError::fail(
            "the rounds wrote no record".to_string(),
        ));
    }
    let base = played.join(journal::BASE);
    let snapshot = if base.is_file() {
        fs::read(&base)
    } else {
        fs::read(played.join(save::FILE))
    }
    .map_err(|err| TestCaseError::fail(err.to_string()))?;
    let pack = TempPack(fresh_dir("prop-rerun-again"));
    copy_dir(played, &pack.0);
    fs::write(pack.0.join(save::FILE), snapshot)
        .map_err(|err| TestCaseError::fail(err.to_string()))?;
    let mut session =
        session::Session::open(&pack.0).map_err(|err| TestCaseError::fail(err.to_string()))?;
    for record in &round_records {
        let effects = round::replay(
            &mut session,
            &record.last_round_at,
            &record.intents,
            &Presence {
                players: record.presence_players,
                actors_present: record.actors_present.clone(),
            },
        )
        .map(|output| output.effects)
        .map_err(|err| {
            TestCaseError::fail(format!("round {} failed: {err}", record.round_index))
        })?;
        if let Some(diff) = replay::find_effect_diff(&record.effects, &effects) {
            return Err(TestCaseError::fail(format!(
                "round {} {diff}",
                record.round_index
            )));
        }
    }
    Ok(())
}
