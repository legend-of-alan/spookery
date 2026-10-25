#[path = "../../spook/tests/support.rs"]
#[allow(dead_code)]
mod support;

#[path = "../../spook/tests/feature_paths.rs"]
mod feature_paths;

#[path = "steps/no_facts.rs"]
mod no_facts;
#[path = "steps/proofs.rs"]
mod proofs;
#[path = "steps/round_play.rs"]
mod round_play;

fn feature_dirs() -> &'static [&'static str] {
    feature_paths::ENGINE_WORLD
}

include!("../../spook/tests/feature_main.rs");
