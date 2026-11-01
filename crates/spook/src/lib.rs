//! The table: seats, wires, and the two terminals.

#[cfg(feature = "client")]
pub use spookery_agent as agent;
#[cfg(feature = "client")]
pub use spookery_cli::{ai_gm, gm, http_model, mcp, play};
pub use spookery_protocol as protocol;
