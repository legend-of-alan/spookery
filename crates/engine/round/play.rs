//! Run a round, commit it, catch the table up, and migrate.

use std::fs;
use std::path::Path;

use crate::commit::CommitOutcome;
use crate::draft::Draft;
use crate::effect::Effect;
use crate::lock::lock_world;
use crate::model::Presence;
use crate::rules::MoveReport;
use crate::visibility::{RecipientLine, ViewerChanges};
use crate::world::World;
use crate::Reason;

use crate::round::stage::advance::{self, plan_catchup};
use crate::round::stage::{apply, complete, gather, observe, record, resolve, triggers};
use crate::round::state::Round;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundCause {
    Scheduler,
    Catchup,
    Flush,
    Manual,
    Replay,
    FastForward,
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

pub(crate) struct RecordedRound {
    pub intents: Vec<crate::intent::Intent>,
    pub presence_humans: u32,
    pub actors_present: Vec<String>,
}

/// Migrate the journal tip. A save that the journal has not passed is migrated as it stands.
pub fn migrate_pack(path: &Path) -> Result<(), Reason> {
    let now = tip_for_migrate(path)?;
    crate::commit::migrate_journal(path, now)
}

fn tip_for_migrate(path: &Path) -> Result<crate::model::Now, Reason> {
    use crate::journal::{read_live, JournalEntry};
    use crate::store::codec::load_save;
    let bytes = fs::read(path.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Reason::Schema)?;
    let file_now = load_save(text).map_err(crate::diagnostic::reason_of)?;
    crate::store::recover::recover(path)?;
    let ahead = read_live(path)
        .map_err(|_| Reason::Schema)?
        .iter()
        .any(|entry| {
            !matches!(entry, JournalEntry::Checkpoint(_))
                && entry.state_rev_after() > file_now.state_rev
        });
    if ahead {
        Ok(crate::round::replay::resume_for_migrate(path)?
            .now()
            .clone())
    } else {
        Ok(file_now)
    }
}

/// Play one round from the save. `Some` is a warning the host prints: the round
/// was committed, and the checkpoint bundle was not closed.
pub fn play_pack_once(path: &Path) -> Result<Option<&'static str>, Reason> {
    let mut world = crate::round::replay::resume(path)?;
    let wall = world.now().last_tick_at.clone();
    play(&mut world, RoundCause::Manual, &wall)?;
    Ok(world.take_warning())
}

pub(crate) fn catch_up(
    world: &mut World,
    now_wall: &str,
    downtime_hours: u64,
) -> Result<(), Reason> {
    let humans = world.now().presence.humans;
    let ticks = plan_catchup(humans, world.budget(), downtime_hours)?;
    for _ in 0..ticks {
        play(world, RoundCause::Catchup, now_wall)?;
    }
    Ok(())
}

/// Keep playing while a seated PC is still busy. The burst starts only when every
/// seated PC is busy. `already` counts a tick this call did not play, and the burst
/// stops at `max_fastforward_ticks` including that tick.
pub(crate) fn advance_while_busy(
    world: &mut World,
    wall: &str,
    already: u32,
) -> Result<Vec<RoundOutput>, Reason> {
    let cap = world.budget().max_fastforward_ticks;
    if !all_seated_busy(world) {
        return Ok(Vec::new());
    }
    let mut ran = already;
    let mut outputs = Vec::new();
    while u64::from(ran) < cap && any_seated_busy(world) && world.queued_intents().is_empty() {
        outputs.push(play(world, RoundCause::FastForward, wall)?);
        ran += 1;
    }
    Ok(outputs)
}

fn all_seated_busy(world: &World) -> bool {
    let pcs = world.now().seated_pcs();
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
    world.now().seated_pcs().iter().any(|id| {
        world
            .now()
            .actors
            .get(id)
            .is_some_and(|actor| actor.is_busy(world.now().world_time))
    })
}

pub(crate) fn play(
    world: &mut World,
    cause: RoundCause,
    now_wall: &str,
) -> Result<RoundOutput, Reason> {
    play_with(world, cause, now_wall, None)
}

pub(crate) fn play_recorded(
    world: &mut World,
    now_wall: &str,
    recorded: RecordedRound,
) -> Result<Vec<Effect>, Reason> {
    play_with(world, RoundCause::Replay, now_wall, Some(recorded)).map(|output| output.effects)
}

fn play_with(
    world: &mut World,
    cause: RoundCause,
    now_wall: &str,
    recorded: Option<RecordedRound>,
) -> Result<RoundOutput, Reason> {
    // A replayed round stays in memory, so only a live round takes the lock.
    let _lock = recorded
        .is_none()
        .then(|| lock_world(world.root()))
        .transpose()?;
    if let Some(recorded) = &recorded {
        world.set_presence(Presence {
            humans: recorded.presence_humans,
            actors_present: recorded.actors_present.clone(),
        });
    }
    let mut draft = Draft::begin(world);
    let mut round = Round::prepare(world, &draft, cause, now_wall, recorded);
    advance::run(&mut round, &mut draft)?;
    complete::run(&mut round, &mut draft)?;
    gather::run(&mut round, &mut draft)?;
    resolve::run(&mut round, &mut draft)?;
    apply::run(&mut round, &mut draft)?;
    triggers::run(&mut round, &mut draft)?;
    observe::run(&mut round, &mut draft)?;
    let output = record::run(world, &mut round, draft)?;
    Ok(drop_unseated_lines(world, output))
}

/// Lines and seen from an empty table are not kept for a later seat.
fn drop_unseated_lines(world: &World, mut output: RoundOutput) -> RoundOutput {
    if world.now().presence.humans == 0 {
        output.lines.clear();
        output.seen.clear();
    }
    output
}
