//! The human player view and the human GM view.

use std::fmt;
use std::io;

mod frame_text;
mod gm;
mod player;

pub(crate) use crate::protocol::refusal_line;
pub use gm::{actor_name, run_gm};
pub use player::run_player;

pub enum Error {
    Message(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Message(reason) => f.write_str(reason),
            Error::Io(err) => err.fmt(f),
        }
    }
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
