//! Move world time forward.

use crate::canon::{Budget, Canon};
use crate::ecs::{self, Presence, WindowCount};
use crate::Reason;

/// Where the round stands: its index and the ticks it spans.
pub(crate) struct Time {
    pub round_index: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
}

pub(crate) fn run(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    wall: &str,
) -> Result<Time, Reason> {
    let round_index = world.round_index + 1;
    let world_before = world.world_time;
    let budget = &canon.budget;
    let ticks = round_ticks(presence.players, budget);
    let world_after = world_before.saturating_add(ticks);
    let windows = [
        (WindowCount::Idle, budget.idle_act_window_ticks),
        (WindowCount::Live, budget.live_act_window_ticks),
    ];
    world.round_index = round_index;
    world.world_time = world_after;
    world.last_round_at = wall.to_string();
    world.actors_mut().expire_modifiers(world_after)?;
    for (count, ticks) in windows {
        if world_before / ticks != world_after / ticks {
            world.actors_mut().reset_window_count(count)?;
        }
    }
    Ok(Time {
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
