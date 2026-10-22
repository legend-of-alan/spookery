//! Queue a player verb until the round commits it.

use spookery_kernel::api::ids::player_intent_id;
use spookery_kernel::api::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_kernel::api::model::ActorLive;
use spookery_kernel::api::rule::{TargetKind, When};
use spookery_kernel::api::rules::gate::{gate, verb_for, GateRefusal};
use spookery_kernel::api::rules::MoveReport;
use spookery_kernel::api::world::World;
use spookery_kernel::api::Reason;
use spookery_round::{advance_while_busy, play, RoundCause, RoundOutput};

use super::frames;
use super::idle_clock::wall_now;
use super::input::{self, canonical_line};

pub struct QueuedMove {
    pub id: String,
    pub actor_id: String,
    pub move_name: String,
}

pub enum MoveOutcome {
    Queued(QueuedMove),
    Committed {
        advanced: bool,
        outputs: Vec<RoundOutput>,
    },
    Busy {
        code: &'static str,
        missing: Option<String>,
        outputs: Vec<RoundOutput>,
    },
}

/// Commit a queued round, then fast-forward while every seated PC is busy.
/// An empty queue only fast-forwards.
pub fn commit_then_fast_forward(world: &mut World) -> Result<Vec<RoundOutput>, Reason> {
    let wall = wall_now()?;
    let mut outputs = Vec::new();
    let committed = !world.queued_intents().is_empty();
    if committed {
        outputs.push(play(world, RoundCause::Flush, &wall)?);
    }
    let already = u32::from(committed);
    outputs.extend(advance_while_busy(world, &wall, already)?);
    Ok(outputs)
}

pub fn handle_player_move(
    world: &mut World,
    actor_id: &str,
    line: &str,
    gm_seated: bool,
) -> Result<MoveOutcome, GateRefusal> {
    let (verb, rest) = input::verb_line(line, actor_id)?;
    let queued = match queue_verb(world, actor_id, verb, rest) {
        Ok(queued) => queued,
        Err(refusal) => {
            if !gm_seated && refusal.reason() == spookery_kernel::api::Reason::Busy {
                let outputs = commit_then_fast_forward(world).map_err(GateRefusal::from)?;
                return Ok(MoveOutcome::Busy {
                    code: refusal.token(),
                    missing: refusal.missing().map(str::to_string),
                    outputs,
                });
            }
            return Err(refusal);
        }
    };
    if gm_seated {
        return Ok(MoveOutcome::Queued(queued));
    }
    let before = world.now().world_time;
    let outputs = commit_then_fast_forward(world)?;
    Ok(MoveOutcome::Committed {
        advanced: world.now().world_time != before,
        outputs,
    })
}

pub fn player_report_lines(reports: &[MoveReport], actor_id: &str) -> String {
    let mut out = String::new();
    for report in reports {
        if report.actor_id() != actor_id {
            continue;
        }
        match report {
            MoveReport::Rolled { out: rolled, .. } => {
                out.push_str(&frames::player_check_line(rolled.outcome))
            }
            MoveReport::Refused { code, missing, .. } => {
                out.push_str(&frames::player_refused(code.token(), missing.as_deref()))
            }
        }
    }
    out
}

pub fn gm_check_frames(reports: &[MoveReport]) -> Vec<Vec<u8>> {
    reports
        .iter()
        .filter_map(|report| match report {
            MoveReport::Rolled { out, .. } => Some(frames::check_event(out)),
            MoveReport::Refused { .. } => None,
        })
        .collect()
}

fn queue_verb(
    world: &mut World,
    actor_id: &str,
    verb: &str,
    rest: &str,
) -> Result<QueuedMove, GateRefusal> {
    if !world
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
    {
        return Err(Reason::Schema.into());
    }
    let kind = verb_target(world, actor_id, verb, rest).ok_or(Reason::NotInSpec)?;
    let target = match kind {
        TargetKind::None => {
            if !rest.is_empty() {
                return Err(Reason::Schema.into());
            }
            Target::Empty
        }
        TargetKind::Actor | TargetKind::Item => {
            let (named, extra) = input::split_token(rest);
            if named.is_empty() || !extra.is_empty() {
                return Err(Reason::Schema.into());
            }
            Target::Actor(named.to_string())
        }
        TargetKind::Place | TargetKind::Exit => {
            let (place, extra) = input::split_token(rest);
            if place.is_empty() || !extra.is_empty() {
                return Err(Reason::Schema.into());
            }
            Target::Place(place.to_string())
        }
    };
    let intent = player_intent(world, actor_id, verb, target)?;
    gate(world.canon(), world.now(), &intent)?;
    let queued = QueuedMove {
        id: intent.id.clone(),
        actor_id: actor_id.to_string(),
        move_name: verb.to_string(),
    };
    world.queue(intent);
    Ok(queued)
}

pub fn queue_wait(world: &mut World, actor_id: &str) -> Result<QueuedMove, GateRefusal> {
    queue_verb(world, actor_id, "wait", "")
}

fn verb_target(world: &World, actor_id: &str, verb: &str, rest: &str) -> Option<TargetKind> {
    let place = world.now().actors.get(actor_id)?.place();
    let (token, _) = input::split_token(rest);
    let hint = (!token.is_empty()).then_some(token);
    let wielded = world
        .now()
        .actors
        .get(actor_id)
        .and_then(|actor| actor.wielded());
    let rule = verb_for(world.canon(), Some(place), verb, hint, wielded)?;
    match &rule.when {
        When::Verb { target, .. } => Some(*target),
        _ => None,
    }
}

fn player_intent(
    world: &World,
    actor_id: &str,
    verb: &str,
    target: Target,
) -> Result<Intent, Reason> {
    let token = match &target {
        Target::Empty => "",
        Target::Actor(id) | Target::Place(id) => id.as_str(),
        Target::Clock(_) => return Err(Reason::Schema),
    };
    let tick = world
        .now()
        .tick_index
        .checked_add(1)
        .ok_or(Reason::Schema)?;
    let id = player_intent_id(
        &world.now().shard_id,
        tick,
        actor_id,
        &canonical_line(verb, token),
    );
    if world.queued_intents().iter().any(|intent| intent.id == id) {
        return Err(Reason::Schema);
    }
    Ok(Intent {
        id,
        tick_index: tick,
        actor_id: actor_id.to_string(),
        actor_kind: ActorKind::Player,
        source: Source::Player,
        move_name: verb.to_string(),
        target,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    })
}

impl QueuedMove {
    pub fn player_line(&self) -> String {
        frames::player_queued_line(&self.id)
    }

    pub fn gm_frame(&self) -> Vec<u8> {
        frames::queued_event(&self.id, &self.actor_id, &self.move_name)
    }
}

pub(super) struct IntentPush<'a> {
    pub id: String,
    pub actor_id: &'a str,
    pub actor_kind: ActorKind,
    pub move_name: &'a str,
    pub target: Target,
    pub args: IntentArgs,
    pub rationale_code: String,
    pub tick_index: Option<u64>,
}

pub(super) fn push_intent(world: &mut World, intent: IntentPush<'_>) -> Result<(), Reason> {
    let tick_index = match intent.tick_index {
        Some(tick_index) => tick_index,
        None => world
            .now()
            .tick_index
            .checked_add(1)
            .ok_or(Reason::Schema)?,
    };
    world.queue(Intent {
        id: intent.id,
        tick_index,
        actor_id: intent.actor_id.to_string(),
        actor_kind: intent.actor_kind,
        source: Source::Player,
        move_name: intent.move_name.to_string(),
        target: intent.target,
        args: intent.args,
        rationale_code: intent.rationale_code,
    });
    Ok(())
}

pub(super) fn mint_intent_id() -> Result<String, Reason> {
    let mut bytes = [0u8; 16];
    if !fill_random(&mut bytes) {
        return Err(Reason::Schema);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

#[cfg(windows)]
fn fill_random(buf: &mut [u8]) -> bool {
    #[link(name = "advapi32")]
    extern "system" {
        fn SystemFunction036(buf: *mut u8, len: u32) -> u8;
    }
    unsafe { SystemFunction036(buf.as_mut_ptr(), buf.len() as u32) != 0 }
}

#[cfg(unix)]
fn fill_random(buf: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(buf))
        .is_ok()
}

#[cfg(not(any(windows, unix)))]
fn fill_random(_buf: &mut [u8]) -> bool {
    false
}
