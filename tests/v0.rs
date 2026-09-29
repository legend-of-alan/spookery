#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "conformance/v0_import_boundaries.rs"]
mod v0_import_boundaries;

#[path = "conformance/v0_t01_closed_secret.rs"]
mod v0_t01_closed_secret;

#[path = "conformance/v0_t02_no_self_win.rs"]
mod v0_t02_no_self_win;

#[path = "conformance/v0_t03_shop_lives.rs"]
mod v0_t03_shop_lives;

#[path = "conformance/v0_t04_catchup_cap.rs"]
mod v0_t04_catchup_cap;

#[path = "conformance/v0_t05_busy_skip.rs"]
mod v0_t05_busy_skip;

#[path = "conformance/v0_t07_replay.rs"]
mod v0_t07_replay;

#[path = "conformance/v0_t08_budget.rs"]
mod v0_t08_budget;

#[path = "conformance/v0_trace.rs"]
mod v0_trace;

#[path = "conformance/v0_scheduler_sleeps_then_catches_up_on_this_thread.rs"]
mod v0_scheduler_sleeps_then_catches_up_on_this_thread;

#[path = "conformance/v0_t06_atomic_commit.rs"]
mod v0_t06_atomic_commit;

#[path = "conformance/v0_t09_tool_scope.rs"]
mod v0_t09_tool_scope;

#[path = "conformance/v0_t10_proposal_only.rs"]
mod v0_t10_proposal_only;

#[path = "conformance/v0_c01_load_mcp.rs"]
mod v0_c01_load_mcp;

#[path = "conformance/v0_canon_schemas_reject_unknown_keys.rs"]
mod v0_canon_schemas_reject_unknown_keys;

#[path = "conformance/v0_pack_rev_mismatch_does_not_commit.rs"]
mod v0_pack_rev_mismatch_does_not_commit;

#[path = "conformance/v0_emitter_sample_is_166_bytes.rs"]
mod v0_emitter_sample_is_166_bytes;

#[path = "conformance/v0_oneshot_docks_matches_section12.rs"]
mod v0_oneshot_docks_matches_section12;

#[path = "conformance/v0_c09_idle_ux.rs"]
mod v0_c09_idle_ux;

#[path = "conformance/v0_c11_strip_bodies.rs"]
mod v0_c11_strip_bodies;

#[path = "conformance/v0_c12_hidden_string_lint.rs"]
mod v0_c12_hidden_string_lint;

#[path = "conformance/v0_cli_status_prints_host_clocks_and_writes_nothing.rs"]
mod v0_cli_status_prints_host_clocks_and_writes_nothing;

#[path = "conformance/v0_cli_migrate.rs"]
mod v0_cli_migrate;
