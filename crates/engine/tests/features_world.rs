#[path = "steps/no_facts.rs"]
mod no_facts;
#[path = "steps/proofs.rs"]
mod proofs;
#[path = "steps/round_play.rs"]
mod round_play;

fn feature_dirs() -> &'static [&'static str] {
    spookery_engine::testing::ENGINE_WORLD
}

include!("feature_main.rs");
