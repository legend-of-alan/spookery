//! The round crate has no cucumber harness. This is the scenario:
//!
//! ```gherkin
//! Feature: Re-run matches the log
//!
//!   Scenario: Running a round's effects again matches the logged effects
//!     Given the docks pack
//!     And a successful grit check adds one lamp oil
//!     When random legal player moves and habit rounds are played
//!     Then running each round again yields the effects written in the journal
//! ```
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use std::fs;
use std::path::Path;

use proptest::test_runner::{TestCaseError, TestRunner};
use spookery_kernel::effect::Effect;
use spookery_kernel::journal::{self, JournalEntry};
use spookery_kernel::store::codec::emit_effect;
use spookery_kernel::world::World;

use crate::play::{play_recorded, RecordedRound};
use crate::prop_play::{
    configure_runner, copy_docks, copy_pack, pick_rounds, play_rounds, TempPack,
};

#[test]
fn running_a_round_again_matches_the_logged_effects() {
    let mut runner = TestRunner::new(configure_runner());
    runner
        .run(&pick_rounds(), |rounds| {
            let played = TempPack(copy_docks("prop-rerun"));
            play_rounds(&played.0, &rounds).map_err(fail_msg)?;
            rerun(&played.0)
        })
        .unwrap_or_else(|err| panic!("re-run property failed: {err}"));
}

fn rerun(played: &Path) -> Result<(), TestCaseError> {
    let records = journal::load_journal(played)
        .map_err(|fault| fail_msg(format!("{}: {}", fault.file, fault.detail)))?;
    let ticks: Vec<_> = records
        .into_iter()
        .filter_map(|entry| match entry {
            JournalEntry::Tick(record) => Some(record),
            _ => None,
        })
        .collect();
    if ticks.is_empty() {
        return Err(fail_msg("the rounds wrote no tick".to_string()));
    }
    let snapshot = fs::read(played.join("state").join(journal::base_rel()))
        .map_err(|err| fail_msg(err.to_string()))?;
    // Same rules as the played copy, including the grit success effect, at the snapshot.
    let pack = TempPack(copy_pack(played, "prop-rerun-again"));
    fs::write(pack.0.join("state/save.yaml"), snapshot).map_err(|err| fail_msg(err.to_string()))?;
    let mut world = World::open(&pack.0).map_err(|err| fail_msg(err.to_string()))?;
    for record in &ticks {
        let effects = play_recorded(
            &mut world,
            &record.last_tick_at,
            RecordedRound {
                intents: record.intents.clone(),
                presence_humans: record.presence_humans,
                actors_present: record.actors_present.clone(),
            },
        )
        .map_err(|err| fail_msg(format!("tick {} did not re-run: {err}", record.tick_index)))?;
        if effects != record.effects {
            return Err(fail_msg(effect_diff(
                record.tick_index,
                &record.effects,
                &effects,
            )));
        }
    }
    Ok(())
}

fn effect_diff(tick: u64, logged: &[Effect], again: &[Effect]) -> String {
    let count = logged.len().max(again.len());
    for index in 0..count {
        let left = logged.get(index);
        let right = again.get(index);
        if left == right {
            continue;
        }
        return format!(
            "tick {tick} effect {index} differed: logged {} again {}",
            effect_text(left),
            effect_text(right)
        );
    }
    format!("tick {tick} effects differed")
}

fn effect_text(effect: Option<&Effect>) -> String {
    match effect {
        Some(effect) => emit_effect(effect),
        None => "none".to_string(),
    }
}

fn fail_msg(message: String) -> TestCaseError {
    TestCaseError::fail(message)
}
