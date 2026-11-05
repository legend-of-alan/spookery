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
    let windows = [
        (ActCounter::IdleActsThisWindow, budget.idle_act_window_ticks),
        (ActCounter::LiveActsThisWindow, budget.live_act_window_ticks),
    ];
    draft.stamp_round(round_index, world_after, wall);
    draft.expire_modifiers(world_after)?;
    // Each act count resets when the round crosses the end of its window.
    for (counter, ticks) in windows {
        if world_before / ticks != world_after / ticks {
            draft.reset_acts(counter)?;
        }
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
