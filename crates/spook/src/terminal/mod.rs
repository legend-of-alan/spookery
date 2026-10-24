//! The human player view and the human GM view.

use std::io;

use spookery_engine::Reason;

mod gm_view;
mod player_view;
mod render;

pub use gm_view::{actor_name, run_gm};
pub use player_view::run_player;
pub(crate) use crate::protocol::refusal;

pub enum Error {
    Reason(Reason),
    Io(io::Error),
}
