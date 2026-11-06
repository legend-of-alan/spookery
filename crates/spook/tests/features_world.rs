//! Cucumber scenarios for the world, the rules, and the pack tools.

#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "mcp_client.rs"]
#[allow(dead_code)]
mod mcp_client;

#[path = "steps/scripted_gm.rs"]
#[allow(dead_code)]
mod scripted_gm;

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

#[path = "steps/rule_names.rs"]
mod rule_names;

#[path = "steps/travel.rs"]
mod travel;

#[path = "steps/lifecycle.rs"]
mod lifecycle;

#[path = "steps/action_triggers.rs"]
mod action_triggers;

#[path = "steps/triggers.rs"]
mod triggers;

#[path = "steps/migrate.rs"]
mod migrate;

#[path = "steps/meters.rs"]
mod meters;

#[path = "steps/stories_and_places.rs"]
mod stories_and_places;

#[path = "steps/modifiers.rs"]
mod modifiers;

#[path = "steps/wake.rs"]
mod wake;

#[path = "steps/sample.rs"]
mod sample;

#[path = "steps/sample_arc.rs"]
mod sample_arc;

fn runs(folder: &str) -> bool {
    !HOST.contains(&folder)
}

include!("feature_main.rs");
