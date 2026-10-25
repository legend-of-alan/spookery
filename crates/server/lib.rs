//! The Spookery table server.

pub mod frames;
mod gm_seat;
mod gm_tools;
pub mod idle_clock;
mod limits;
mod listen;
mod mcp_http;
pub mod starter;
mod world_loop;

pub use gm_tools::call_gm_tool;
pub use idle_clock::on_scheduled_wake;
pub use limits::{HELLO_LIMIT, OUTBOUND_BYTES, OUTBOUND_FRAMES, REQUESTED_SEND_BUFFER};
pub use listen::{door_request, serve, Config, Error, Handle};
pub use spookery_protocol as protocol;
pub use world_loop::{commit_then_fast_forward, MoveOutcome};
