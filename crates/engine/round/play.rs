//! Run a round, commit it, and catch the table up.

use std::path::Path;

use crate::canon::Budget;
use crate::commit::CommitOutcome;
use crate::draft::Draft;
use crate::effect::Effect;
use crate::intent::Intent;
use crate::lock::lock_pack;
use crate::model::Presence;
use crate::rules::MoveReport;
use crate::visibility::{RecipientLine, ViewerChanges};
use crate::world::World;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;
use crate::round::stage::advance::RoundTime;
use crate::round::stage::{advance, apply, deliver, finish, pick, record, resolve, triggers};

/// Returned from the engine's round to the host table, which sends it on to players, the GM and the digest.
/// What one round showed. Lines and seen are empty when no human was seated.
#[derive(Default)]
pub struct RoundOutput {
    pub reports: Vec<MoveReport>,
    pub dice: Vec<String>,
    pub summary: Vec<String>,
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

/// Play the rounds `hours` of absence earn, at an empty table. A short absence plays one.
pub(crate) fn play_catch_up(world: &mut World, wall: &str, hours: u64) -> Result<(), Reason> {
    let rounds = if hours == 0 {
        1
    } else {
        planned_rounds(world.budget(), hours)?.max(1)
    };
    let empty = Presence::default();
    for _ in 0..rounds {
        play_round(world, &empty, wall, false)?;
    }
    Ok(())
}

/// How many rounds `hours` of empty-table absence earn, within the budget's caps.
fn planned_rounds(budget: &Budget, hours: u64) -> Result<u64, Reason> {
    let every = budget.round_every_real_seconds;
    let idle_minutes = budget.idle_minutes_per_round;
    if every == 0 || idle_minutes == 0 {
        return Err(Reason::Schema);
    }
    let absence_minutes = hours.checked_mul(60).ok_or(Reason::OutOfRange)?;
    let absence_seconds = hours.checked_mul(3600).ok_or(Reason::OutOfRange)?;
    let printed = absence_seconds.div_ceil(every);
    let minute_cap = budget
        .idle_max_world_hours_per_real_hour
        .checked_mul(absence_minutes)
        .ok_or(Reason::OutOfRange)?;
    let by_minutes = minute_cap / idle_minutes;
    Ok(printed.min(budget.idle_catchup_max_rounds).min(by_minutes))
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
    let now = world.state();
    presence
        .actors_present
        .iter()
        .map(|id| {
            now.actors
                .get(id)
                .is_some_and(|actor| actor.is_busy(now.world_time))
        })
        .collect()
}

/// Play the queue and commit the round. Only a World that owns the pack lock can.
pub(crate) fn play_round(
    world: &mut World,
    presence: &Presence,
    wall: &str,
    fast_forward: bool,
) -> Result<RoundOutput, Reason> {
    let (draft, clock, log, delivered) = run_stages(world, presence, wall, world.queued_intents())?;
    record::run(world, draft, wall, clock, log, delivered, fast_forward)
}

/// Play the queue a journal line kept, in memory, with the seats it recorded.
pub(crate) fn replay_round(
    world: &mut World<()>,
    wall: &str,
    queue: &[Intent],
    presence: &Presence,
) -> Result<RoundOutput, Reason> {
    let (draft, _clock, mut log, delivered) = run_stages(world, presence, wall, queue)?;
    let output = record::take_output(&mut log, delivered);
    crate::commit::keep_in_memory(world, draft)?;
    Ok(output)
}

fn run_stages<'p, L>(
    world: &World<L>,
    presence: &'p Presence,
    wall: &str,
    queue: &[Intent],
) -> Result<(Draft<'p>, RoundTime, RoundLog, deliver::Deliveries), Reason> {
    let mut draft = Draft::begin(world, presence);
    let clock = advance::run(&mut draft, wall)?;
    let mut dice = RoundDice::new(draft.state().world_id.clone(), clock.round_index);
    let mut log = RoundLog::default();
    let candidates = pick::run(&draft, queue)?;
    let snapshot = draft.state().clone();
    let resolved = resolve::run(
        draft.canon(),
        &snapshot,
        presence,
        candidates,
        queue,
        &mut dice,
        &mut log,
    )?;
    apply::run(&mut draft, &snapshot, resolved, &clock, &mut dice, &mut log)?;
    finish::run(&mut draft, clock.world_time_after, &mut dice, &mut log)?;
    let changes = triggers::run(&mut draft, &mut dice, &mut log)?;
    let delivered = deliver::run(
        draft.canon(),
        draft.state(),
        draft.presence,
        &changes,
        &log.effects,
    );
    Ok((draft, clock, log, delivered))
}
