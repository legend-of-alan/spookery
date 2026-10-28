//! Move world time forward.

use crate::canon::tick_settings::Budget;
use crate::draft::Draft;
use crate::Reason;

use crate::round::state::Round;

pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    let world_before = draft.now().world_time;
    draft.stamp_round(round.round_index, world_before, &round.now_wall);
    let minutes = minutes_for(draft.now().presence.humans, draft.budget());
    let world_after = world_before.saturating_add(minutes);
    let day_changed = world_before / 1440 != world_after / 1440;
    let hour_changed = world_before / 60 != world_after / 60;
    draft.stamp_round(round.round_index, world_after, &round.now_wall);
    round.world_time_after = world_after;
    if day_changed {
        draft.reset_idle_acts()?;
    }
    if hour_changed {
        draft.reset_live_acts()?;
    }
    draft.apply_pause_policy()
}

fn minutes_for(humans: u32, budget: &Budget) -> u64 {
    if humans > 0 {
        budget.live_minutes_per_tick
    } else {
        budget.idle_minutes_per_tick
    }
}
