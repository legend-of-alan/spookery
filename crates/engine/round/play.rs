//! Run a round, commit it, and catch the table up.

use std::path::Path;
use std::sync::Arc;

use crate::canon::{Budget, Canon};
use crate::ecs::{self, component, Presence};
use crate::intent::Intent;
use crate::rules::effect::Effect;
use crate::rules::roll::MoveReport;
use crate::rules::travel;
use crate::rules::visibility::{RecipientLine, ViewerChanges};
use crate::session;
use crate::store::commit::CommitOutcome;
use crate::store::lock::lock_pack;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::stage::advance::RoundTime;
use crate::round::stage::{advance, apply, deliver, finish, pick, record, resolve, triggers};
use crate::round::Ledger;

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

/// Play one round from the save.
pub fn play_pack_once(path: &Path) -> Result<(), Reason> {
    let mut session = session::Session::resume(path, lock_pack(path)?)?;
    let wall = session.world().last_round_at.clone();
    play_round(&mut session, &Presence::default(), &wall, false)?;
    Ok(())
}

/// Play the rounds `real_hours` of absence earn, at an empty table. A short absence plays one.
pub(crate) fn play_catch_up(
    session: &mut session::Session,
    wall: &str,
    real_hours: u64,
) -> Result<(), Reason> {
    let rounds = if real_hours == 0 {
        1
    } else {
        planned_rounds(session.budget(), real_hours)?.max(1)
    };
    let empty = Presence::default();
    for _ in 0..rounds {
        play_round(session, &empty, wall, false)?;
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
    session: &mut session::Session,
    presence: &Presence,
    wall: &str,
    already: u32,
) -> Result<Vec<RoundOutput>, Reason> {
    let cap = session.budget().max_fastforward_rounds;
    let busy = seated_busy(session, presence);
    if busy.is_empty() || busy.contains(&false) {
        return Ok(Vec::new());
    }
    let mut ran = already;
    let mut outputs = Vec::new();
    while u64::from(ran) < cap
        && seated_busy(session, presence).contains(&true)
        && session.queued_intents().is_empty()
    {
        outputs.push(play_round(session, presence, wall, true)?);
        ran += 1;
    }
    Ok(outputs)
}

/// Whether each seated PC is busy at the current world time.
fn seated_busy<Lock>(session: &session::Session<Lock>, presence: &Presence) -> Vec<bool> {
    let (world, actors) = (session.world(), session.world().actors());
    presence
        .actors_present
        .iter()
        .map(|id| actors.get::<component::Act>(id).is_busy(world.world_time))
        .collect()
}

/// Play the queue and commit the round. Only a session that owns the pack lock can.
pub(crate) fn play_round(
    session: &mut session::Session,
    presence: &Presence,
    wall: &str,
    fast_forward: bool,
) -> Result<RoundOutput, Reason> {
    let canon = Arc::clone(session.world.canon());
    let world = session.begin_round();
    let staged = run_stages(&canon, world, presence, wall, session.queued_intents())?;
    record::run(session, staged, presence, wall, fast_forward)
}

/// Play the queue a journal line kept, in memory, with the seats it recorded.
pub(crate) fn replay_round(
    session: &mut session::Session<()>,
    wall: &str,
    queue: &[Intent],
    presence: &Presence,
) -> Result<RoundOutput, Reason> {
    let canon = Arc::clone(session.world.canon());
    let world = session.begin_round();
    let StagedRound {
        world,
        mut ledger,
        delivered,
        ..
    } = run_stages(&canon, world, presence, wall, queue)?;
    let output = record::take_output(&mut ledger, delivered);
    crate::store::commit::keep_in_memory(session, world)?;
    Ok(output)
}

/// What the stages leave for `record`: the round's working copy of the save, its time, its ledger,
/// and what each seat hears.
pub(crate) struct StagedRound {
    pub(crate) world: ecs::World,
    pub(crate) clock: RoundTime,
    pub(crate) ledger: Ledger,
    pub(crate) delivered: deliver::Deliveries,
}

/// Every stage but `record`, in order, on `world`, the round's working copy of the save. The
/// candidates' rules borrow the canon, not the save, while the later stages write it.
fn run_stages(
    canon: &Canon,
    mut world: ecs::World,
    presence: &Presence,
    wall: &str,
    queue: &[Intent],
) -> Result<StagedRound, Reason> {
    let clock = advance::run(canon, &mut world, presence, wall)?;
    let mut dice = RoundDice::new(world.seed.clone(), clock.round_index);
    let mut ledger = Ledger::default();
    let picked = pick::run(canon, &world, presence, queue, &mut ledger)?;
    let candidates = resolve::run(
        canon,
        &world,
        presence,
        &picked,
        queue,
        &mut dice,
        &mut ledger,
    );
    apply::run(
        canon,
        &mut world,
        presence,
        candidates,
        &clock,
        &mut dice,
        &mut ledger,
    );
    let end = clock.world_time_after;
    finish::run(canon, &mut world, presence, end, &mut dice, &mut ledger)?;
    // A door that opens is a flag change, so change rules see it before the map refresh. This
    // pass also takes the clock's changes in a round where no action ran.
    travel::open_doors(canon, &mut world, presence, &mut ledger);
    triggers::run(canon, &mut world, presence, &mut dice, &mut ledger);
    travel::note_exits(canon, &mut world)?;
    let delivered = deliver::run(canon, &world, presence, &ledger.changes, &ledger.effects);
    Ok(StagedRound {
        world,
        clock,
        ledger,
        delivered,
    })
}
