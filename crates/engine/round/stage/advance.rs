//! Move world time forward.

use crate::canon::Budget;
use crate::draft::Draft;
use crate::model::{ActCounter, MINUTES_PER_DAY, MINUTES_PER_HOUR};
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
    let day_changed = world_before / MINUTES_PER_DAY != world_after / MINUTES_PER_DAY;
    let hour_changed = world_before / MINUTES_PER_HOUR != world_after / MINUTES_PER_HOUR;
    draft.stamp_round(round_index, world_after, wall);
    if day_changed {
        draft.reset_acts(ActCounter::IdleActsToday)?;
    }
    if hour_changed {
        draft.reset_acts(ActCounter::LiveActsThisHour)?;
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
