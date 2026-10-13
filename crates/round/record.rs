use spookery_kernel::world::{Draft, World};
use spookery_kernel::Reason;

use crate::log::{render_tick, TickLine};
use crate::play::TickCause;
use crate::round::Round;
use crate::tick_seed::tick_seed;

pub fn record(world: &mut World, round: &mut Round, draft: Draft) -> Result<(), Reason> {
    round.trace.push("record".to_string());
    if round.owns_commit {
        round.trace.push("tick_commit".to_string());
    }
    let seed = tick_seed(&round.world_id, round.tick_index);
    let jsonl = render_tick(&TickLine {
        tick_index: round.tick_index,
        state_rev_before: round.rev_before,
        state_rev_after: round.rev_before + 1,
        world_time_before: round.world_time_before,
        world_time_after: round.world_time_after,
        presence_humans: round.humans,
        seed: &seed,
        last_tick_at: &round.now_wall,
        intents: &round.applied,
        rejected: &round.rejected,
        effects: &round.effects,
        tool_trace: &round.trace,
    });
    world.finish_tick(
        draft,
        round.cause == TickCause::Flush,
        jsonl,
        std::mem::take(&mut round.notices),
        round.recorded.is_some(),
    )
}
