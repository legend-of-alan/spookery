use cucumber::when;

use crate::World;

#[when(expr = "the empty tick log is checked")]
fn the_empty_tick_log_is_checked(_world: &mut World) {
    crate::trace::empty_tick_log();
}

#[when(expr = "the flushed wait log is checked")]
fn the_flushed_wait_log_is_checked(_world: &mut World) {
    crate::trace::flushed_wait_log();
}

#[when(expr = "the budget tick log is checked")]
fn the_budget_tick_log_is_checked(_world: &mut World) {
    crate::trace::budget_tick_log();
}

#[when(expr = "the catch-up tick log is checked")]
fn the_catchup_tick_log_is_checked(_world: &mut World) {
    crate::trace::catchup_tick_log();
}

#[when(expr = "the schemas are checked for unknown keys")]
fn the_schemas_are_checked(_world: &mut World) {
    crate::canon_schemas::schemas_reject_unknown_keys();
}

#[when(expr = "the now sample is emitted")]
fn the_now_sample_is_emitted(_world: &mut World) {
    crate::emitter_sample::now_sample_emits_the_recorded_bytes();
}

#[when(expr = "the docks save is checked")]
fn the_docks_save_is_checked(_world: &mut World) {
    crate::oneshot_docks::docks_save_round_trips();
}

#[when(expr = "the scheduler sleeps and catches up")]
async fn the_scheduler_sleeps(_world: &mut World) {
    crate::scheduler::scheduler_sleeps_then_catches_up().await;
}
