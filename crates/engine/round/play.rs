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
use crate::round::stage::{advance, apply, deliver, finish, pick, record, resolve, triggers};

pub(crate) enum RoundKind<'a> {
    /// Drain the queue and write the journal line.
    Live { fast_forward: bool },
    /// Play the queue a journal line kept, in memory, with the seats it recorded.
    Replay {
        queue: &'a [Intent],
        presence: Presence,
    },
}

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
    pub outcome: Option<CommitOutcome>,
}

/// Play one round from the save. `Some` is a warning the host prints: the round
/// was committed, and the checkpoint bundle was not closed.
pub fn play_pack_once(path: &Path) -> Result<Option<&'static str>, Reason> {
    let _lock = lock_pack(path)?;
    let mut world = crate::round::replay::open_at_tip(path)?;
    let wall = world.now().last_round_at.clone();
    play_round(
        &mut world,
        &wall,
        RoundKind::Live {
            fast_forward: false,
        },
    )?;
    Ok(world.take_warning())
}

/// Play the rounds `hours` of absence earn. A seated table or a short absence plays one.
pub(crate) fn play_catch_up(world: &mut World, wall: &str, hours: u64) -> Result<(), Reason> {
    let rounds = if hours == 0 || world.now().presence.humans > 0 {
        1
    } else {
        planned_rounds(world.budget(), hours)?.max(1)
    };
    for _ in 0..rounds {
        play_round(
            world,
            wall,
            RoundKind::Live {
                fast_forward: false,
            },
        )?;
    }
    Ok(())
}

/// How many rounds `hours` of empty-table absence earn, within the budget's caps.
fn planned_rounds(budget: &Budget, hours: u64) -> Result<u64, Reason> {
    let every = budget.tick_every_real_seconds;
    let idle_minutes = budget.idle_minutes_per_tick;
    if every == 0 || idle_minutes == 0 {
        return Err(Reason::Schema);
    }
    let absence_minutes = hours.checked_mul(60).ok_or(Reason::OutOfRange)?;
    let absence_seconds = hours.checked_mul(3600).ok_or(Reason::OutOfRange)?;
    let printed = ceil_div(absence_seconds, every);
    let minute_cap = budget
        .idle_max_world_hours_per_real_hour
        .checked_mul(absence_minutes)
        .ok_or(Reason::OutOfRange)?;
    let by_minutes = minute_cap / idle_minutes;
    Ok(printed.min(budget.idle_catchup_max_ticks).min(by_minutes))
}

fn ceil_div(numerator: u64, denominator: u64) -> u64 {
    if numerator == 0 {
        return 0;
    }
    let quotient = numerator / denominator;
    if numerator.is_multiple_of(denominator) {
        quotient
    } else {
        quotient.saturating_add(1)
    }
}

/// Keep playing while a seated PC is still busy. The burst starts only when every
/// seated PC is busy. `already` counts a round this call did not play, and the burst
/// stops at `max_fastforward_ticks` including that round.
pub(crate) fn play_while_busy(
    world: &mut World,
    wall: &str,
    already: u32,
) -> Result<Vec<RoundOutput>, Reason> {
    let cap = world.budget().max_fastforward_ticks;
    let busy = seated_busy(world);
    if busy.is_empty() || busy.contains(&false) {
        return Ok(Vec::new());
    }
    let mut ran = already;
    let mut outputs = Vec::new();
    while u64::from(ran) < cap
        && seated_busy(world).contains(&true)
        && world.queued_intents().is_empty()
    {
        outputs.push(play_round(
            world,
            wall,
            RoundKind::Live { fast_forward: true },
        )?);
        ran += 1;
    }
    Ok(outputs)
}

/// Whether each seated PC is busy at the current world time.
fn seated_busy(world: &World) -> Vec<bool> {
    let now = world.now();
    now.seated_pcs()
        .iter()
        .map(|id| {
            now.actors
                .get(id)
                .is_some_and(|actor| actor.is_busy(now.world_time))
        })
        .collect()
}

pub(crate) fn play_round(
    world: &mut World,
    wall: &str,
    kind: RoundKind<'_>,
) -> Result<RoundOutput, Reason> {
    if let RoundKind::Replay { presence, .. } = &kind {
        world.set_presence_inner(presence.clone());
    }
    let mut draft = Draft::begin(world);
    let clock = advance::run(&mut draft, wall)?;
    let mut dice = RoundDice::new(draft.now().shard_id.clone(), clock.round_index);
    let mut log = RoundLog::default();
    let queue = match &kind {
        RoundKind::Live { .. } => world.queued_intents(),
        RoundKind::Replay { queue, .. } => queue,
    };
    let candidates = pick::run(&draft, queue)?;
    let snapshot = draft.now().clone();
    let resolved = resolve::run(
        draft.canon(),
        &snapshot,
        candidates,
        queue,
        &mut dice,
        &mut log,
    )?;
    apply::run(&mut draft, &snapshot, resolved, &clock, &mut dice, &mut log)?;
    finish::run(&mut draft, clock.world_time_after, &mut dice, &mut log)?;
    let changes = triggers::run(&mut draft, &mut dice, &mut log)?;
    let delivered = deliver::run(draft.canon(), draft.now(), &changes, &log.effects);
    record::run(world, draft, &kind, wall, clock, log, delivered)
}
