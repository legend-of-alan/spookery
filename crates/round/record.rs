use spookery_kernel::world::{Draft, World};
use spookery_kernel::Reason;

use crate::log::{render_round_line, RoundLine};
use crate::play::RoundCause;
use crate::round::Round;
use crate::round_seed::round_seed;

pub(crate) fn run(world: &mut World, round: &mut Round, draft: Draft) -> Result<(), Reason> {
    round.trace.push("record".to_string());
    if round.owns_commit {
        round.trace.push("commit_round".to_string());
    }
    let seed = round_seed(&round.world_id, round.tick_index);
    let jsonl = render_round_line(&RoundLine {
        tick_index: round.tick_index,
        rules_rev: spookery_kernel::store::RULES_REV,
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
        arrivals: &round.arrivals,
        fast_forward: round.fast_forward,
        tool_trace: &round.trace,
    });
    world.finish_round(
        draft,
        round.cause == RoundCause::Flush,
        jsonl,
        std::mem::take(&mut round.notices),
        round.recorded.is_some(),
    )
}
