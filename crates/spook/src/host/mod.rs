//! Listen, seat players, and run the world.

pub mod frames;
mod gm_seat;
mod gm_tools;
pub mod idle_clock;
mod limits;
mod listen;
mod mcp_http;
mod world_loop;

pub use gm_tools::call_gm_tool;
pub use idle_clock::on_scheduled_wake;
pub use limits::{OUTBOUND_BYTES, OUTBOUND_FRAMES, REQUESTED_SEND_BUFFER};
pub(crate) use listen::door_request;
pub use listen::{listen, Error};
pub use world_loop::{commit_then_fast_forward, handle_player_move, MoveOutcome};
