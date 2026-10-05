#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "conformance/v0/trace.rs"]
mod trace;

#[path = "conformance/v0/canon_schemas.rs"]
mod canon_schemas;

#[path = "conformance/v0/emitter_sample.rs"]
mod emitter_sample;

#[path = "conformance/v0/oneshot_docks.rs"]
mod oneshot_docks;

#[path = "conformance/v0/scheduler.rs"]
mod scheduler;
