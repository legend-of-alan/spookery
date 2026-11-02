//! The plain integration tests, linked as one program.
//! A change in the library relinks this once. Feature scenarios run in
//! `features_host` and `features_world`.

#[path = "../support.rs"]
#[allow(dead_code)]
mod support;

#[path = "../feature_dirs.rs"]
mod feature_dirs;

mod engine_guarantees;
mod feature_census;
mod live_round;
mod pack_reference;
mod seat_protocol;
