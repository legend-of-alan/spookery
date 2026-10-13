use std::path::Path;

use spookery_kernel::lock::lock_world;
use spookery_kernel::world::{Draft, World};
use spookery_kernel::Reason;

use crate::advance::{self, plan_catchup};
use crate::apply;
use crate::complete;
use crate::gather;
use crate::observe;
use crate::record;
use crate::resolve;
use crate::round::Round;
use crate::run_triggers;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundCause {
    Scheduler,
    Catchup,
    Flush,
    Manual,
    Replay,
}

pub struct RecordedRound {
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
    play(world, RoundCause::Flush, wall)?;
    spookery_kernel::commit::commit_pending(world)
}

pub fn open_pack(path: &Path) -> Result<(), Reason> {
    let mut world = World::open(path)?;
    let wall = world.now().last_tick_at.clone();
    play(&mut world, RoundCause::Manual, &wall)
}

pub fn catch_up(world: &mut World, now_wall: &str, downtime_hours: u64) -> Result<(), Reason> {
    let humans = spookery_kernel::world::session_status(world.now()).humans;
    let ticks = plan_catchup(humans, world.budget(), downtime_hours)?;
    for _ in 0..ticks {
        play(world, RoundCause::Catchup, now_wall)?;
    }
    Ok(())
}

pub fn play(world: &mut World, cause: RoundCause, now_wall: &str) -> Result<(), Reason> {
    play_with(world, cause, now_wall, None)
}

pub(crate) fn play_recorded(
    world: &mut World,
    now_wall: &str,
    recorded: RecordedRound,
) -> Result<(), Reason> {
    play_with(world, RoundCause::Replay, now_wall, Some(recorded))
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
    cause: RoundCause,
    now_wall: &str,
    recorded: Option<RecordedRound>,
) -> Result<(), Reason> {
    let _lock = if cause == RoundCause::Flush {
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
        advance::run(&mut round, draft)?;
        complete::run(&mut round, draft)?;
        gather::run(&mut round, draft)?;
        resolve::run(&mut round, draft)?;
        apply::run(&mut round, draft)?;
        run_triggers::run(&mut round, draft)?;
        observe::run(&mut round, draft)?;
        if pass == passes {
            record::run(world, &mut round, draft_slot.take().expect("draft"))?;
            return Ok(());
        }
    }
    Ok(())
}
