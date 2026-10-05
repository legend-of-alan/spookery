use rmcp as _;
use spookery_kernel as _;
use tokio as _;

pub mod profile;
pub mod tick_tools;

pub use profile::{parse_profile, serve, Profile};
