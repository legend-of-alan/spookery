//! Cucumber scenarios for the world, the rules, and the pack tools.

#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "steps/table.rs"]
#[allow(dead_code)]
mod table;

#[path = "steps/common.rs"]
mod common;

#[path = "steps/tool_trace.rs"]
mod tool_trace;

#[path = "steps/rounds.rs"]
mod rounds;

#[path = "steps/round_play.rs"]
mod round_play;

#[path = "steps/round_layout.rs"]
mod round_layout;

#[path = "steps/replay.rs"]
mod replay;

#[path = "steps/rules.rs"]
mod rules;

#[path = "steps/items.rs"]
mod items;

#[path = "steps/checks.rs"]
mod checks;

#[path = "steps/check.rs"]
mod check;

#[path = "steps/rows.rs"]
mod rows;

#[path = "steps/travel.rs"]
mod travel;

#[path = "steps/triggers.rs"]
mod triggers;

#[path = "steps/commit.rs"]
mod commit;

#[path = "steps/kernel_boundary.rs"]
mod kernel_boundary;

#[path = "steps/proofs.rs"]
mod proofs;

#[path = "steps/fixtures.rs"]
mod fixtures;

#[path = "steps/audit.rs"]
mod audit;

#[path = "steps/sample.rs"]
mod sample;

#[path = "steps/contributing.rs"]
mod contributing;

#[path = "steps/readme.rs"]
mod readme;

#[path = "fixture/oneshot_docks.rs"]
mod oneshot_docks;

#[path = "feature_paths.rs"]
mod feature_paths;

fn feature_dirs() -> &'static [&'static str] {
    feature_paths::WORLD
}

fn finish_scenario() {}

include!("feature_main.rs");
