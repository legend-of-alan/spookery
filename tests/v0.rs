#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "conformance/v0/t01_closed_secret.rs"]
mod t01_closed_secret;

#[path = "conformance/v0/t02_no_self_win.rs"]
mod t02_no_self_win;

#[path = "conformance/v0/t03_shop_lives.rs"]
mod t03_shop_lives;

#[path = "conformance/v0/t04_catchup_cap.rs"]
mod t04_catchup_cap;

#[path = "conformance/v0/t05_busy_skip.rs"]
mod t05_busy_skip;

#[path = "conformance/v0/t06_atomic_commit.rs"]
mod t06_atomic_commit;

#[path = "conformance/v0/t07_replay.rs"]
mod t07_replay;

#[path = "conformance/v0/t08_budget.rs"]
mod t08_budget;

#[path = "conformance/v0/t09_tool_scope.rs"]
mod t09_tool_scope;

#[path = "conformance/v0/t10_proposal_only.rs"]
mod t10_proposal_only;

#[path = "conformance/v0/trace.rs"]
mod trace;

#[path = "conformance/v0/c01_load_mcp.rs"]
mod c01_load_mcp;

#[path = "conformance/v0/c09_idle_ux.rs"]
mod c09_idle_ux;

#[path = "conformance/v0/c11_strip_bodies.rs"]
mod c11_strip_bodies;

#[path = "conformance/v0/c12_hidden_string_lint.rs"]
mod c12_hidden_string_lint;

#[path = "conformance/v0/cli_migrate.rs"]
mod cli_migrate;

#[path = "conformance/v0/cli_status.rs"]
mod cli_status;

#[path = "conformance/v0/canon_schemas.rs"]
mod canon_schemas;

#[path = "conformance/v0/emitter_sample.rs"]
mod emitter_sample;

#[path = "conformance/v0/import_boundaries.rs"]
mod import_boundaries;

#[path = "conformance/v0/oneshot_docks.rs"]
mod oneshot_docks;

#[path = "conformance/v0/pack_rev.rs"]
mod pack_rev;

#[path = "conformance/v0/scheduler.rs"]
mod scheduler;
