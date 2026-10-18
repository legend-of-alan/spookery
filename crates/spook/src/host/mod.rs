//! Listen, seat players, and run the world.

pub(crate) mod frames;
mod gm_seat;
mod gm_tools;
pub mod idle_clock;
mod input;
mod limits;
mod listen;
mod mcp_http;
mod player_moves;
mod speech;
mod world_loop;

pub use gm_tools::{call_gm_tool, GmToolReply};
pub use idle_clock::on_scheduled_wake;
pub use limits::{OUTBOUND_BYTES, OUTBOUND_FRAMES, REQUESTED_SEND_BUFFER};
pub use listen::{listen, Error};
pub use player_moves::{commit_queued, handle_player_move, queue_wait, MoveOutcome};
pub use speech::{player_scrollback, speak};
pub use world_loop::{release_player_seat, seat_player, SocketEnd};
