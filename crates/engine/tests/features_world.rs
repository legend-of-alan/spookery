#[path = "steps/docks.rs"]
mod docks;
#[path = "steps/no_facts.rs"]
mod no_facts;
#[path = "steps/read_lock.rs"]
mod read_lock;
#[path = "steps/round_play.rs"]
mod round_play;

fn feature_dirs() -> &'static [&'static str] {
    spookery_engine::testing::WORLD
}

include!("feature_main.rs");
