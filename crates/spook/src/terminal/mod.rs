//! The human player view and the human GM view.

use std::io;

use spookery_kernel::api::Reason;

mod gm_view;
mod player_view;
mod render;

pub use gm_view::run_gm_terminal;
pub use player_view::run_player_terminal;

pub enum TerminalError {
    Reason(Reason),
    Io(io::Error),
}
