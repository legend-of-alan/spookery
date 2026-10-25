#[path = "../../spook/tests/support.rs"]
#[allow(dead_code)]
mod support;

#[path = "../../spook/tests/feature_paths.rs"]
mod feature_paths;

#[path = "steps/matters.rs"]
mod matters;

fn feature_dirs() -> &'static [&'static str] {
    feature_paths::ENGINE_HOST
}

include!("../../spook/tests/feature_main.rs");
