//! The Spookery table server.

mod frames;
mod gm_seat;
mod gm_tools;
pub mod idle_clock;
mod limits;
mod listener;
mod mcp_http;
mod outbound;
mod world_loop;

pub use frames::resume;
pub use gm_tools::call_gm_tool;
pub use limits::{OUTBOUND_BYTE_CAP, OUTBOUND_FRAME_CAP, REQUESTED_SEND_BUFFER};
pub use listener::{serve, Config, Error};
use spookery_protocol as protocol;
pub use world_loop::commit_then_fast_forward;
