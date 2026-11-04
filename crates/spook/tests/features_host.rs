//! Cucumber scenarios for the host, the seats, and the terminal.

#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "mcp_client.rs"]
#[allow(dead_code)]
mod mcp_client;

#[path = "steps/table.rs"]
#[allow(dead_code)]
mod table;

#[path = "steps/common.rs"]
mod common;

#[path = "steps/ports.rs"]
mod ports;

#[path = "steps/status.rs"]
mod status;

#[path = "steps/speech.rs"]
mod speech;

#[path = "steps/seats.rs"]
mod seats;

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

#[path = "steps/ai_gm.rs"]
mod ai_gm;

#[path = "steps/round_output.rs"]
mod round_output;

#[path = "steps/mcp.rs"]
mod mcp;

#[path = "steps/new.rs"]
mod new;

#[path = "steps/gm_terminal.rs"]
mod gm_terminal;

#[path = "steps/refusal.rs"]
mod refusal;

#[path = "steps/one_writer.rs"]
mod one_writer;

#[path = "steps/session.rs"]
mod session;

#[path = "steps/websocket.rs"]
mod websocket;

#[path = "steps/claims.rs"]
mod claims;

#[path = "steps/store_failures.rs"]
mod store_failures;

#[path = "steps/recorded_session.rs"]
mod recorded_session;

fn runs(folder: &str) -> bool {
    HOST.contains(&folder)
}

include!("feature_main.rs");
