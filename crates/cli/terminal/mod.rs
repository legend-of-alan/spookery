//! The human player view and the human GM view.

use std::io;

mod gm_view;
mod player_view;
mod render;

pub(crate) use crate::protocol::refusal;
pub use gm_view::{actor_name, run_gm};
pub use player_view::run_player;

pub enum Error {
    Reason(String),
    Io(io::Error),
}

/// Stdin lines from a plain thread. A read on `tokio::io::stdin` can't be
/// cancelled, so the runtime would wait on exit for the next Enter.
fn stdin_lines() -> tokio::sync::mpsc::UnboundedReceiver<String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in io::stdin().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    rx
}
