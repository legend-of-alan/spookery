#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "steps/matters.rs"]
mod matters;

fn feature_dirs() -> &'static [&'static str] {
    &["tests/features/seats"]
}

include!("feature_main.rs");
