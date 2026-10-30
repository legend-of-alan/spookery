//! Cucumber scenarios for the world, the rules, and the pack tools.

#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "steps/table.rs"]
#[allow(dead_code)]
mod table;

#[path = "steps/common.rs"]
mod common;

#[path = "steps/record.rs"]
mod record;

#[path = "steps/rounds.rs"]
mod rounds;

#[path = "steps/replay.rs"]
mod replay;

#[path = "steps/rules.rs"]
mod rules;

#[path = "steps/items.rs"]
mod items;

#[path = "steps/checks.rs"]
mod checks;

#[path = "steps/expressions.rs"]
mod expressions;

#[path = "steps/spook_check.rs"]
mod spook_check;

#[path = "steps/missing_item.rs"]
mod missing_item;

#[path = "steps/travel.rs"]
mod travel;

#[path = "steps/triggers.rs"]
mod triggers;

#[path = "steps/migrate.rs"]
mod migrate;

#[path = "steps/wake.rs"]
mod wake;

#[path = "steps/sample.rs"]
mod sample;

#[path = "steps/sample_arc.rs"]
mod sample_arc;

#[path = "feature_dirs.rs"]
mod feature_dirs;

fn feature_dirs() -> &'static [&'static str] {
    feature_dirs::WORLD
}

include!("feature_main.rs");
