//! The table: seats, wires, and the two terminals.

#[cfg(feature = "client")]
pub use spookery_cli::ai;
#[cfg(feature = "client")]
pub use spookery_cli::{gm_connection, terminal};
pub use spookery_protocol as protocol;
#[cfg(feature = "server")]
pub use spookery_server as host;
#[cfg(feature = "server")]
pub use spookery_server::{starter, Error};
