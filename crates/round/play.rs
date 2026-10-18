//! Run a round, commit it, catch the table up, and migrate.

use std::fs;
use std::path::Path;

use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::effect::Effect;
use spookery_kernel::api::lock::lock_world;
use spookery_kernel::api::model::Presence;
use spookery_kernel::api::world::World;
use spookery_kernel::api::Reason;

use crate::stage::advance::{self, plan_catchup};
use crate::stage::{apply, complete, gather, observe, record, resolve, triggers};
use crate::state::Round;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundCause {
    Scheduler,
    Catchup,
    Flush,
    Manual,
    Replay,
    FastForward,
}

pub struct RecordedRound {
    pub intents: Vec<spookery_kernel::api::intent::Intent>,
    pub presence_humans: u32,
    pub actors_present: Vec<String>,
}

pub fn commit_moves(
    world: &mut World,
) -> Result<spookery_kernel::api::commit::CommitOutcome, Reason> {
    let wall = world.now().last_tick_at.clone();
    commit_moves_at(world, &wall)
}

/// `include_tick: true` with `last_tick_at` set to `wall`. The caller supplies the stamp.
pub fn commit_moves_at(
    world: &mut World,
    wall: &str,
) -> Result<spookery_kernel::api::commit::CommitOutcome, Reason> {
    let _lock = lock_world(world.root())?;
    play(world, RoundCause::Flush, wall)?;
    spookery_kernel::api::commit::commit_pending(world)
}

/// Migrate the journal tip. A save that the journal has not passed is migrated as it stands.
pub fn migrate_pack(path: &Path) -> Result<(), Reason> {
    let now = tip_for_migrate(path)?;
    spookery_kernel::api::commit::migrate_now(path, now)
}

fn tip_for_migrate(path: &Path) -> Result<spookery_kernel::api::model::Now, Reason> {
    use spookery_kernel::api::journal::{read_live, JournalEntry};
    use spookery_kernel::api::store::codec::load_save;
    let bytes = fs::read(path.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Reason::Schema)?;
    let file_now = load_save(text).map_err(spookery_kernel::api::diagnostic::reason_of)?;
    let ahead = read_live(path)
        .map_err(|_| Reason::Schema)?
        .iter()
        .any(|entry| {
            !matches!(entry, JournalEntry::Checkpoint(_))
                && entry.state_rev_after() > file_now.state_rev
        });
    if ahead {
        Ok(crate::replay::resume(path)?.now().clone())
    } else {
        Ok(file_now)
    }
}

/// Play one round from the save. `Some` is a warning the host prints: the round
/// was committed, and the checkpoint bundle was not closed.
pub fn play_pack_once(path: &Path) -> Result<Option<&'static str>, Reason> {
    let mut world = crate::replay::resume(path)?;
    let wall = world.now().last_tick_at.clone();
    play(&mut world, RoundCause::Manual, &wall)?;
    Ok(world.take_warning())
}

pub fn catch_up(world: &mut World, now_wall: &str, downtime_hours: u64) -> Result<(), Reason> {
    let humans = spookery_kernel::api::world::session_status(world.now()).humans;
    let ticks = plan_catchup(humans, world.budget(), downtime_hours)?;
    for _ in 0..ticks {
        play(world, RoundCause::Catchup, now_wall)?;
    }
    Ok(())
}

/// Keep playing while a seated PC is still busy. The burst starts only when every
/// seated PC is busy. `already` counts a tick this call did not play, and the burst
/// stops at `max_fastforward_ticks` including that tick.
pub fn advance_while_busy(world: &mut World, wall: &str, already: u32) -> Result<u32, Reason> {
    let cap = world.budget().max_fastforward_ticks;
    if !all_seated_busy(world) {
        return Ok(already);
    }
    let mut ran = already;
    while u64::from(ran) < cap && any_seated_busy(world) && world.queued_intents().is_empty() {
        play(world, RoundCause::FastForward, wall)?;
        ran += 1;
    }
    Ok(ran)
}

fn seated_pcs(world: &World) -> Vec<String> {
    world
        .now()
        .presence
        .actors_present
        .iter()
        .filter(|id| {
            world
                .now()
                .actors
                .get(id.as_str())
                .is_some_and(spookery_kernel::api::model::ActorLive::is_pc)
        })
        .cloned()
        .collect()
}

fn all_seated_busy(world: &World) -> bool {
    let pcs = seated_pcs(world);
    !pcs.is_empty()
        && pcs.iter().all(|id| {
            world
                .now()
                .actors
                .get(id)
                .is_some_and(|actor| actor.is_busy(world.now().world_time))
        })
}

fn any_seated_busy(world: &World) -> bool {
    seated_pcs(world).iter().any(|id| {
        world
            .now()
            .actors
            .get(id)
            .is_some_and(|actor| actor.is_busy(world.now().world_time))
    })
}

pub fn play(world: &mut World, cause: RoundCause, now_wall: &str) -> Result<(), Reason> {
    play_with(world, cause, now_wall, None).map(|_| ())
}

pub(crate) fn play_recorded(
    world: &mut World,
    now_wall: &str,
    recorded: RecordedRound,
) -> Result<Vec<Effect>, Reason> {
    play_with(world, RoundCause::Replay, now_wall, Some(recorded))
}

fn play_with(
    world: &mut World,
    cause: RoundCause,
    now_wall: &str,
    recorded: Option<RecordedRound>,
) -> Result<Vec<Effect>, Reason> {
    let _lock = if cause == RoundCause::Flush {
        None
    } else {
        Some(lock_world(world.root())?)
    };
    if let Some(recorded) = &recorded {
        world.set_presence(Presence {
            humans: recorded.presence_humans,
            actors_present: recorded.actors_present.clone(),
        });
    }
    let mut draft_slot = Some(Draft::begin(world));
    let mut round = Round::prepare(
        world,
        draft_slot.as_ref().expect("draft"),
        cause,
        now_wall,
        recorded,
    );
    let draft = draft_slot.as_mut().expect("draft");
    advance::run(&mut round, draft)?;
    complete::run(&mut round, draft)?;
    gather::run(&mut round, draft)?;
    resolve::run(&mut round, draft)?;
    apply::run(&mut round, draft)?;
    triggers::run(&mut round, draft)?;
    observe::run(&mut round, draft)?;
    record::run(world, &mut round, draft_slot.take().expect("draft"))?;
    Ok(std::mem::take(&mut round.effects))
}
