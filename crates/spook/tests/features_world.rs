//! Cucumber scenarios for the world, the rules, and the pack tools.

#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "steps/table.rs"]
#[allow(dead_code)]
mod table;

#[path = "steps/common.rs"]
mod common;

#[path = "steps/steps_run.rs"]
mod steps_run;

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

#[path = "steps/numbers.rs"]
mod numbers;

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

#[path = "steps/leak_audit.rs"]
mod leak_audit;

#[path = "steps/sample.rs"]
mod sample;

#[path = "steps/no_facts.rs"]
mod no_facts;

#[path = "steps/contributing.rs"]
mod contributing;

#[path = "steps/module_names.rs"]
mod module_names;

#[path = "steps/module_docs.rs"]
mod module_docs;

#[path = "steps/readme.rs"]
mod readme;

#[path = "fixture/oneshot_docks.rs"]
mod oneshot_docks;

#[path = "feature_paths.rs"]
mod feature_paths;

fn feature_dirs() -> &'static [&'static str] {
    feature_paths::WORLD
}

include!("feature_main.rs");
