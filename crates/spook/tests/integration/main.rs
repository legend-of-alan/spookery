//! The plain integration tests, linked as one program.
//! A change in the library relinks this once. The cucumber runner stays `promises`.

#[path = "../support.rs"]
#[allow(dead_code)]
mod support;

mod engine_guarantees;
mod live_round;
mod pack_reference;
mod seat_protocol;
