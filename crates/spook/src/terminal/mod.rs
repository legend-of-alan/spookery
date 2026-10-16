use std::io;

use spookery_kernel::api::Reason;

mod gm;
mod player;
mod render;

pub use gm::run_gm_terminal;
pub use player::run_player_terminal;

pub enum TerminalError {
    Reason(Reason),
    Io(io::Error),
}
