#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "conformance/v0/trace.rs"]
mod trace;

#[path = "conformance/v0/c01_load_mcp.rs"]
mod c01_load_mcp;

#[path = "conformance/v0/c09_idle_ux.rs"]
mod c09_idle_ux;

#[path = "conformance/v0/cli_migrate.rs"]
mod cli_migrate;

#[path = "conformance/v0/cli_status.rs"]
mod cli_status;

#[path = "conformance/v0/canon_schemas.rs"]
mod canon_schemas;

#[path = "conformance/v0/emitter_sample.rs"]
mod emitter_sample;

#[path = "conformance/v0/oneshot_docks.rs"]
mod oneshot_docks;

#[path = "conformance/v0/scheduler.rs"]
mod scheduler;
