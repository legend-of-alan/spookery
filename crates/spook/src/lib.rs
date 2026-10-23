//! The table: seats, wires, and the two terminals.
//!
//! Public API
//! - `ai`: the model client and the GM loop that calls it.
//! - `gm_connection`: the client that sits the GM seat over the agent door.
//! - `host`: listen, seat players, and run the world.
//! - `protocol`: the player wire, the GM wire, and the check band they share.
//! - `terminal`: the human player view and the human GM view.
//!
//! Internal
//!
//! None. Every module of this crate is public.

pub mod ai;
pub mod gm_connection;
pub mod host;
pub use spookery_protocol as protocol;
pub mod terminal;

pub use host::Error;
