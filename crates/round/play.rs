use std::path::Path;

use spookery_kernel::lock::lock_world;
use spookery_kernel::world::{Draft, World};
use spookery_kernel::Reason;

use crate::advance::plan_catchup;
use crate::round::Round;
use crate::stages;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickCause {
    Scheduler,
    Catchup,
    Flush,
    Manual,
    Replay,
}

pub struct RecordedTick {
    pub intents: Vec<spookery_kernel::intent::Intent>,
    pub presence_humans: u32,
}

/// Off until W1. One pass today.
const FAST_FORWARD: bool = false;

pub fn commit_moves(world: &mut World) -> Result<spookery_kernel::commit::CommitOutcome, Reason> {
    let wall = world.now().last_tick_at.clone();
    commit_moves_at(world, &wall)
}

/// `include_tick: true` with `last_tick_at` set to `wall`. The caller supplies the stamp.
pub fn commit_moves_at(
    world: &mut World,
    wall: &str,
) -> Result<spookery_kernel::commit::CommitOutcome, Reason> {
    let _lock = lock_world(world)?;
    play(world, TickCause::Flush, wall)?;
    spookery_kernel::commit::commit_pending(world)
}

pub fn play_pack(path: &Path) -> Result<(), Reason> {
    let mut world = World::open(path)?;
    let wall = world.now().last_tick_at.clone();
    play(&mut world, TickCause::Manual, &wall)
}

pub fn play_catchup(world: &mut World, now_wall: &str, downtime_hours: u64) -> Result<(), Reason> {
    let humans = spookery_kernel::world::session_status(world.now()).humans;
    let ticks = plan_catchup(humans, world.budget(), downtime_hours)?;
    for _ in 0..ticks {
        play(world, TickCause::Catchup, now_wall)?;
    }
    Ok(())
}

pub fn play(world: &mut World, cause: TickCause, now_wall: &str) -> Result<(), Reason> {
    play_with(world, cause, now_wall, None)
}

pub(crate) fn play_recorded(
    world: &mut World,
    now_wall: &str,
    recorded: RecordedTick,
) -> Result<(), Reason> {
    play_with(world, TickCause::Replay, now_wall, Some(recorded))
}

fn stage_passes() -> u32 {
    if FAST_FORWARD {
        64
    } else {
        1
    }
}

fn play_with(
    world: &mut World,
    cause: TickCause,
    now_wall: &str,
    recorded: Option<RecordedTick>,
) -> Result<(), Reason> {
    let _lock = if cause == TickCause::Flush {
        None
    } else {
        Some(lock_world(world)?)
    };
    let passes = stage_passes();
    let mut draft_slot = Some(Draft::begin(world));
    let mut round = Round::prepare(
        world,
        draft_slot.as_ref().expect("draft"),
        cause,
        now_wall,
        recorded,
    );
    for pass in 1..=passes {
        let draft = draft_slot.as_mut().expect("draft");
        stages::advance(&mut round, draft)?;
        stages::complete(&mut round, draft)?;
        stages::gather(&mut round, draft)?;
        stages::resolve(&mut round, draft)?;
        stages::apply(&mut round, draft)?;
        stages::run_triggers(&mut round, draft)?;
        stages::observe(&mut round, draft)?;
        if pass == passes {
            stages::record(world, &mut round, draft_slot.take().expect("draft"))?;
            return Ok(());
        }
    }
    Ok(())
}
