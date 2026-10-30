//! The Spookery table server.

mod frames;
mod gm_seat;
mod gm_tools;
mod idle_clock;
mod limits;
mod listener;
mod mcp_http;
mod outbound;
mod table;

pub use listener::{serve, Config, Error};
use spookery_protocol as protocol;
