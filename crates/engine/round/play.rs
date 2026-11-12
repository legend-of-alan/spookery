//! Run a round, commit it, and catch the table up.

use std::path::Path;
use std::sync::Arc;

use crate::canon::{Budget, Canon};
use crate::intent::Intent;
use crate::rules::effect::Effect;
use crate::rules::roll::MoveReport;
use crate::rules::travel;
use crate::rules::visibility::{RecipientLine, ViewerChanges};
use crate::state::{component, Presence, State};
use crate::store::commit::CommitOutcome;
use crate::store::lock::lock_pack;
use crate::world::World;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;
use crate::round::stage::advance::RoundTime;
use crate::round::stage::{advance, apply, deliver, finish, pick, record, resolve, triggers};

/// What one round showed. Returned from the engine's round to the host table, which sends it on
/// to players, the GM and the digest. `lines` and `seen` are empty when no human was seated.
pub struct RoundOutput {
    pub reports: Vec<MoveReport>,
    pub dice: Vec<String>,
    pub changed_keys: Vec<String>,
    pub lines: Vec<RecipientLine>,
    pub seen: Vec<ViewerChanges>,
    pub(crate) effects: Vec<Effect>,
    /// Set when this round wrote the journal.
    pub commit: Option<CommitOutcome>,
}

/// Play one round from the save. `Some` is a warning the host prints: the round
/// was committed, and the checkpoint bundle was not closed.
pub fn play_pack_once(path: &Path) -> Result<Option<&'static str>, Reason> {
    let mut world = World::resume(path, lock_pack(path)?)?;
    let wall = world.state().last_round_at.clone();
    play_round(&mut world, &Presence::default(), &wall, false)?;
    Ok(world.take_warning())
}

/// Play the rounds `real_hours` of absence earn, at an empty table. A short absence plays one.
pub(crate) fn play_catch_up(world: &mut World, wall: &str, real_hours: u64) -> Result<(), Reason> {
    let rounds = if real_hours == 0 {
        1
    } else {
        planned_rounds(world.budget(), real_hours)?.max(1)
    };
    let empty = Presence::default();
    for _ in 0..rounds {
        play_round(world, &empty, wall, false)?;
    }
    Ok(())
}

/// How many rounds `real_hours` of empty-table absence earn, within the budget's caps.
fn planned_rounds(budget: &Budget, real_hours: u64) -> Result<u64, Reason> {
    let every = budget.round_every_real_seconds;
    let idle_ticks = budget.idle_ticks_per_round;
    if every == 0 || idle_ticks == 0 {
        return Err(Reason::Schema);
    }
    let absence_seconds = real_hours.checked_mul(3600).ok_or(Reason::OutOfRange)?;
    let printed = absence_seconds.div_ceil(every);
    let tick_cap = budget
        .idle_max_ticks_per_real_hour
        .checked_mul(real_hours)
        .ok_or(Reason::OutOfRange)?;
    let by_ticks = tick_cap / idle_ticks;
    Ok(printed.min(budget.idle_catchup_max_rounds).min(by_ticks))
}

/// Keep playing while a seated PC is still busy. The burst starts only when every
/// seated PC is busy. `already` counts a round this call did not play, and the burst
/// stops at `max_fastforward_rounds` including that round.
pub(crate) fn fast_forward(
    world: &mut World,
    presence: &Presence,
    wall: &str,
    already: u32,
) -> Result<Vec<RoundOutput>, Reason> {
    let cap = world.budget().max_fastforward_rounds;
    let busy = seated_busy(world, presence);
    if busy.is_empty() || busy.contains(&false) {
        return Ok(Vec::new());
    }
    let mut ran = already;
    let mut outputs = Vec::new();
    while u64::from(ran) < cap
        && seated_busy(world, presence).contains(&true)
        && world.queued_intents().is_empty()
    {
        outputs.push(play_round(world, presence, wall, true)?);
        ran += 1;
    }
    Ok(outputs)
}

/// Whether each seated PC is busy at the current world time.
fn seated_busy<L>(world: &World<L>, presence: &Presence) -> Vec<bool> {
    let state = world.state();
    presence
        .actors_present
        .iter()
        .map(|id| state.actors().get::<component::Act>(id).get())
        .map(|act| act.is_some_and(|act| act.is_busy(state.world_time)))
        .collect()
}

/// Play the queue and commit the round. Only a World that owns the pack lock can.
pub(crate) fn play_round(
    world: &mut World,
    presence: &Presence,
    wall: &str,
    fast_forward: bool,
) -> Result<RoundOutput, Reason> {
    let canon = Arc::clone(world.state.canon());
    let state = world.begin_round();
    let staged = run_stages(&canon, state, presence, wall, world.queued_intents())?;
    record::run(world, staged, presence, wall, fast_forward)
}

/// Play the queue a journal line kept, in memory, with the seats it recorded.
pub(crate) fn replay_round(
    world: &mut World<()>,
    wall: &str,
    queue: &[Intent],
    presence: &Presence,
) -> Result<RoundOutput, Reason> {
    let canon = Arc::clone(world.state.canon());
    let state = world.begin_round();
    let StagedRound {
        state,
        mut log,
        delivered,
        ..
    } = run_stages(&canon, state, presence, wall, queue)?;
    let output = record::take_output(&mut log, delivered);
    crate::store::commit::keep_in_memory(world, state)?;
    Ok(output)
}

/// What the stages leave for `record`: the round's working copy of the save, its time, its log,
/// and what each seat hears.
pub(crate) struct StagedRound {
    pub(crate) state: State,
    pub(crate) clock: RoundTime,
    pub(crate) log: RoundLog,
    pub(crate) delivered: deliver::Deliveries,
}

/// Every stage but `record`, in order, on `state`, the round's working copy of the save. The
/// candidates' rules borrow the canon, not the save, while the later stages write it.
fn run_stages(
    canon: &Canon,
    mut state: State,
    presence: &Presence,
    wall: &str,
    queue: &[Intent],
) -> Result<StagedRound, Reason> {
    let clock = advance::run(canon, &mut state, presence, wall)?;
    let mut dice = RoundDice::new(state.world_id.clone(), clock.round_index);
    let mut log = RoundLog::default();
    let picked = pick::run(canon, &state, presence, queue, &mut log)?;
    let candidates = resolve::run(canon, &state, presence, &picked, queue, &mut dice, &mut log);
    apply::run(
        canon, &mut state, presence, candidates, &clock, &mut dice, &mut log,
    );
    let end = clock.world_time_after;
    finish::run(canon, &mut state, presence, end, &mut dice, &mut log)?;
    // A door that opens is a flag change, so change rules see it before the map refresh. This
    // pass also takes the clock's changes in a round where no action ran.
    travel::open_doors(canon, &mut state, presence, &mut log);
    triggers::run(canon, &mut state, presence, &mut dice, &mut log);
    travel::note_exits(canon, &mut state)?;
    let delivered = deliver::run(canon, &state, presence, &log.changes, &log.effects);
    Ok(StagedRound {
        state,
        clock,
        log,
        delivered,
    })
}
