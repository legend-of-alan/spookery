//! Move world time forward.

use crate::canon::Budget;
use crate::draft::Draft;
use crate::Reason;

/// Where the round's clock stands: its index and the world minutes it spans.
pub(crate) struct RoundTime {
    pub round_index: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
}

pub(crate) fn run(draft: &mut Draft, wall: &str) -> Result<RoundTime, Reason> {
    let round_index = draft.state().round_index + 1;
    let world_before = draft.state().world_time;
    let minutes = minutes_for(draft.presence.players, draft.budget());
    let world_after = world_before.saturating_add(minutes);
    let day_changed = world_before / 1440 != world_after / 1440;
    let hour_changed = world_before / 60 != world_after / 60;
    draft.stamp_round(round_index, world_after, wall);
    if day_changed {
        draft.reset_idle_acts()?;
    }
    if hour_changed {
        draft.reset_live_acts()?;
    }
    draft.apply_pause_policy()?;
    Ok(RoundTime {
        round_index,
        world_time_before: world_before,
        world_time_after: world_after,
    })
}

fn minutes_for(players: u32, budget: &Budget) -> u64 {
    if players > 0 {
        budget.live_minutes_per_round
    } else {
        budget.idle_minutes_per_round
    }
}
