//! The table: seats, wires, and the two terminals.

pub mod host;
pub use host::Error;
pub use spookery_cli::{ai, gm_connection, terminal};
pub use spookery_protocol as protocol;
