// cargo +stable-x86_64-pc-windows-gnu test --test bdd --offline -- --name "Twelve empty ticks leave assassination_debt unchanged"
#[path = "support.rs"]
#[allow(dead_code)]
mod support;

use cucumber::World as _;

#[allow(dead_code)]
pub struct PortGuard(support::PortLock);

impl std::fmt::Debug for PortGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PortGuard")
    }
}

#[derive(Debug, Default, cucumber::World)]
pub struct World {
    pub pack: Option<std::path::PathBuf>,
    pub outputs: Vec<std::process::Output>,
    pub text: String,
    pub port: Option<PortGuard>,
    pub scratch: std::collections::BTreeMap<String, String>,
}

#[tokio::main]
async fn main() {
    World::cucumber()
        .max_concurrent_scenarios(1)
        .fail_on_skipped()
        .run_and_exit("tests/features")
        .await;
}

#[path = "step_definitions/v0_no_self_win_steps.rs"]
mod v0_no_self_win_steps;

#[path = "step_definitions/v0_closed_secret_steps.rs"]
mod v0_closed_secret_steps;
#[path = "step_definitions/v0_shop_lives_steps.rs"]
mod v0_shop_lives_steps;
#[path = "step_definitions/v0_catchup_cap_steps.rs"]
mod v0_catchup_cap_steps;
#[path = "step_definitions/v0_busy_skip_steps.rs"]
mod v0_busy_skip_steps;
#[path = "step_definitions/v0_atomic_commit_steps.rs"]
mod v0_atomic_commit_steps;
#[path = "step_definitions/v0_replay_steps.rs"]
mod v0_replay_steps;
#[path = "step_definitions/v0_budget_steps.rs"]
mod v0_budget_steps;
#[path = "step_definitions/v0_tool_scope_steps.rs"]
mod v0_tool_scope_steps;
#[path = "step_definitions/v0_proposal_only_steps.rs"]
mod v0_proposal_only_steps;
#[path = "step_definitions/v0_traces_steps.rs"]
mod v0_traces_steps;
#[path = "step_definitions/v0_load_mcp_steps.rs"]
mod v0_load_mcp_steps;
#[path = "step_definitions/v0_idle_ux_steps.rs"]
mod v0_idle_ux_steps;
#[path = "step_definitions/v0_strip_bodies_steps.rs"]
mod v0_strip_bodies_steps;
#[path = "step_definitions/v0_hidden_string_lint_steps.rs"]
mod v0_hidden_string_lint_steps;
#[path = "step_definitions/v0_import_boundaries_steps.rs"]
mod v0_import_boundaries_steps;
#[path = "step_definitions/v0_pack_rev_steps.rs"]
mod v0_pack_rev_steps;
#[path = "step_definitions/v0_emitter_sample_steps.rs"]
mod v0_emitter_sample_steps;
#[path = "step_definitions/v0_canon_schemas_steps.rs"]
mod v0_canon_schemas_steps;
#[path = "step_definitions/v0_cli_status_steps.rs"]
mod v0_cli_status_steps;
#[path = "step_definitions/v0_cli_migrate_steps.rs"]
mod v0_cli_migrate_steps;
#[path = "step_definitions/v0_oneshot_docks_steps.rs"]
mod v0_oneshot_docks_steps;
#[path = "step_definitions/v0_scheduler_steps.rs"]
mod v0_scheduler_steps;
#[path = "step_definitions/v01_seats_steps.rs"]
mod v01_seats_steps;
#[path = "step_definitions/v01_seat_drop_steps.rs"]
mod v01_seat_drop_steps;
#[path = "step_definitions/v01_bind_steps.rs"]
mod v01_bind_steps;
#[path = "step_definitions/v01_stdin_and_crash_steps.rs"]
mod v01_stdin_and_crash_steps;
#[path = "step_definitions/v01_speech_steps.rs"]
mod v01_speech_steps;
#[path = "step_definitions/v01_round_queue_steps.rs"]
mod v01_round_queue_steps;
#[path = "step_definitions/v01_round_restart_steps.rs"]
mod v01_round_restart_steps;
#[path = "step_definitions/v01_attempt_steps.rs"]
mod v01_attempt_steps;
#[path = "step_definitions/v01_gm_allowlist_steps.rs"]
mod v01_gm_allowlist_steps;
#[path = "step_definitions/v01_gm_say_steps.rs"]
mod v01_gm_say_steps;
#[path = "step_definitions/v01_stdio_bridge_steps.rs"]
mod v01_stdio_bridge_steps;
#[path = "step_definitions/v01_typed_gm_steps.rs"]
mod v01_typed_gm_steps;
#[path = "step_definitions/v02_tool_keys_steps.rs"]
mod v02_tool_keys_steps;
#[path = "step_definitions/v02_pack_table_list_only_steps.rs"]
mod v02_pack_table_list_only_steps;
#[path = "step_definitions/v02_travel_queue_steps.rs"]
mod v02_travel_queue_steps;
#[path = "step_definitions/v02_reconnect_steps.rs"]
mod v02_reconnect_steps;
#[path = "step_definitions/v02_wire_codec_steps.rs"]
mod v02_wire_codec_steps;
#[path = "step_definitions/v02_action_row_load_steps.rs"]
mod v02_action_row_load_steps;
#[path = "step_definitions/v02_flag_queue_steps.rs"]
mod v02_flag_queue_steps;
#[path = "step_definitions/v02_unknown_flag_steps.rs"]
mod v02_unknown_flag_steps;
#[path = "step_definitions/v02_street_steps.rs"]
mod v02_street_steps;
#[path = "step_definitions/v02_check_and_lists_steps.rs"]
mod v02_check_and_lists_steps;
#[path = "step_definitions/v02_opposing_check_steps.rs"]
mod v02_opposing_check_steps;
#[path = "step_definitions/v02_missing_item_steps.rs"]
mod v02_missing_item_steps;
