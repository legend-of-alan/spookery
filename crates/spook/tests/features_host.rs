//! Cucumber scenarios for the host, the seats, and the terminal.

#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "steps/table.rs"]
#[allow(dead_code)]
mod table;

#[path = "steps/common.rs"]
mod common;

#[path = "steps/ports.rs"]
mod ports;

#[path = "steps/harness.rs"]
mod harness;

#[path = "steps/speech.rs"]
mod speech;

#[path = "steps/seats.rs"]
mod seats;

#[path = "steps/place.rs"]
mod place;

#[path = "steps/limits.rs"]
mod limits;

#[path = "steps/loopback.rs"]
mod loopback;

#[path = "steps/cli.rs"]
mod cli;

#[path = "steps/digest.rs"]
mod digest;

#[path = "steps/host.rs"]
mod host;

#[path = "steps/mcp.rs"]
mod mcp;

#[path = "steps/new.rs"]
mod new;

#[path = "steps/gm.rs"]
mod gm;

#[path = "feature_paths.rs"]
mod feature_paths;

fn feature_dirs() -> &'static [&'static str] {
    feature_paths::HOST
}

fn finish_scenario() {
    mcp::drop_holder();
}

include!("feature_main.rs");
