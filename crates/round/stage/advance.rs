use spookery_kernel::api::canon::world_settings::Budget;
use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::pause::apply_pause_policy;
use spookery_kernel::api::Reason;

use crate::state::Round;

pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    round.steps_run.push("advance".to_string());
    let world_before = draft.now().world_time;
    round.world_time_before = world_before;
    round.rev_before = draft.now().state_rev;
    round.tick_index = draft.now().tick_index + 1;
    round.world_id = draft.now().shard_id.clone();
    draft.pin_loop(round.tick_index, world_before, &round.now_wall);
    let minutes = minutes_for(round.humans, draft.budget());
    let world_after = world_before.saturating_add(minutes);
    let day_changed = world_before / 1440 != world_after / 1440;
    let hour_changed = world_before / 60 != world_after / 60;
    draft.pin_loop(round.tick_index, world_after, &round.now_wall);
    round.world_time_after = world_after;
    if day_changed {
        draft.reset_idle_acts()?;
    }
    if hour_changed {
        draft.reset_live_acts()?;
    }
    apply_pause_policy(draft)
}

pub fn minutes_for(humans: u32, budget: &Budget) -> u64 {
    if humans > 0 {
        budget.live_minutes_per_tick
    } else {
        budget.idle_minutes_per_tick
    }
}

/// How many ticks to run for `downtime_hours` of real absence.
/// A seated table runs one tick. An empty table is capped.
pub fn plan_catchup(humans: u32, budget: &Budget, downtime_hours: u64) -> Result<u64, Reason> {
    if humans > 0 {
        return Ok(1);
    }
    let every = budget.tick_every_real_seconds;
    let idle_minutes = budget.idle_minutes_per_tick;
    if every == 0 || idle_minutes == 0 {
        return Err(Reason::Schema);
    }
    let per_hour = downtime_hours.checked_mul(60).ok_or(Reason::OutOfRange)?;
    let printed = ceil_div(per_hour, every);
    let minute_cap = budget
        .idle_max_world_hours_per_real_hour
        .checked_mul(per_hour)
        .ok_or(Reason::OutOfRange)?;
    let by_minutes = minute_cap / idle_minutes;
    Ok(printed.min(budget.idle_catchup_max_ticks).min(by_minutes))
}

fn ceil_div(numerator: u64, denominator: u64) -> u64 {
    if numerator == 0 {
        return 0;
    }
    let quotient = numerator / denominator;
    if numerator.is_multiple_of(denominator) {
        quotient
    } else {
        quotient.saturating_add(1)
    }
}
