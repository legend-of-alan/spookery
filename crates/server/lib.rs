//! The Spookery table server.

mod frames;
mod gm_seat;
mod gm_tools;
pub mod idle_clock;
mod limits;
mod listen;
mod mcp_http;
mod outbound;
pub mod starter;
mod world_loop;

pub use frames::player_resume_line;
pub use gm_tools::call_gm_tool;
pub use limits::{OUTBOUND_BYTES, OUTBOUND_FRAMES, REQUESTED_SEND_BUFFER};
pub use listen::{serve, Config, Error};
use spookery_protocol as protocol;
pub use world_loop::commit_then_fast_forward;
