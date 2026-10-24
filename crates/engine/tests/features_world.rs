#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "steps/no_facts.rs"]
mod no_facts;
#[path = "steps/proofs.rs"]
mod proofs;
#[path = "steps/round_play.rs"]
mod round_play;
#[path = "steps/steps_run.rs"]
mod steps_run;

fn feature_dirs() -> &'static [&'static str] {
    &[
        "tests/features/catchup",
        "tests/features/clocks",
        "tests/features/commit",
        "tests/features/firewall",
        "tests/features/perceive",
        "tests/features/round",
        "tests/features/rows",
        "tests/features/sample",
        "tests/features/stock",
    ]
}

include!("feature_main.rs");
