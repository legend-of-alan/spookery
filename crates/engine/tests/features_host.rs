#[path = "steps/changes.rs"]
mod changes;

fn feature_dirs() -> &'static [&'static str] {
    spookery_engine::testing::HOST
}

include!("feature_main.rs");
