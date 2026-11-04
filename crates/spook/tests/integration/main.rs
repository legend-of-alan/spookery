//! The plain integration tests, linked as one program.
//! A change in the library relinks this once. Feature scenarios run in
//! `features_host` and `features_world`.

#[path = "../support.rs"]
#[allow(dead_code)]
mod support;

#[path = "../mcp_client.rs"]
#[allow(dead_code)]
mod mcp_client;

mod engine_guarantees;
mod live_round;
mod pack_reference;
mod seat_protocol;
