//! Move world time forward.

use crate::canon::Budget;
use crate::draft::Draft;
use crate::model::ActCounter;
use crate::Reason;

/// Where the round stands: its index and the ticks it spans.
pub(crate) struct RoundTime {
    pub round_index: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
}

pub(crate) fn run(draft: &mut Draft, wall: &str) -> Result<RoundTime, Reason> {
    let round_index = draft.state().round_index + 1;
    let world_before = draft.state().world_time;
    let budget = draft.budget();
    let ticks = round_ticks(draft.presence.players, budget);
    let world_after = world_before.saturating_add(ticks);
    let day_changed = world_before / budget.ticks_per_day != world_after / budget.ticks_per_day;
    let window = budget.live_act_window_ticks;
    let window_changed = world_before / window != world_after / window;
    draft.stamp_round(round_index, world_after, wall);
    draft.expire_modifiers(world_after)?;
    if day_changed {
        draft.reset_acts(ActCounter::IdleActsToday)?;
    }
    if window_changed {
        draft.reset_acts(ActCounter::LiveActsThisWindow)?;
    }
    Ok(RoundTime {
        round_index,
        world_time_before: world_before,
        world_time_after: world_after,
    })
}

fn round_ticks(players: u32, budget: &Budget) -> u64 {
    if players > 0 {
        budget.live_ticks_per_round
    } else {
        budget.idle_ticks_per_round
    }
}
