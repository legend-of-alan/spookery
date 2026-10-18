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

#[path = "steps/player_speech.rs"]
mod player_speech;

#[path = "steps/seats.rs"]
mod seats;

#[path = "steps/matters.rs"]
mod matters;

#[path = "steps/place.rs"]
mod place;

#[path = "steps/listener_limits.rs"]
mod listener_limits;

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

#[path = "steps/refusal.rs"]
mod refusal;

#[path = "feature_paths.rs"]
mod feature_paths;

fn feature_dirs() -> &'static [&'static str] {
    feature_paths::HOST
}

include!("feature_main.rs");
