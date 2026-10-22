//! The human player view and the human GM view.

use std::io;

use spookery_kernel::api::Reason;

mod gm_view;
mod player_view;
mod render;

pub use gm_view::{actor_name, run_gm};
pub use player_view::run_player;

pub enum Error {
    Reason(Reason),
    Io(io::Error),
}
