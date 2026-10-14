use spookery_kernel::ids::player_intent_id;
use spookery_kernel::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_kernel::model::ActorLive;
use spookery_kernel::rules::gate::{gate, verb_for, Miss};
use spookery_kernel::rules::rule::{TargetKind, When};
use spookery_kernel::rules::RollReport;
use spookery_kernel::world::World;
use spookery_kernel::Reason;
use spookery_round::commit_moves_at;

use super::frames;
use super::idle_clock::wall_now;
use super::input::{self, canonical_line};

pub struct QueuedNote {
    pub id: String,
    pub actor_id: String,
    pub move_name: String,
}

pub enum MoveOutcome {
    Queued(QueuedNote),
    Committed {
        advanced: bool,
        notices: Vec<RollReport>,
    },
}

pub fn handle_player_move(
    world: &mut World,
    actor_id: &str,
    line: &str,
    gm_seated: bool,
) -> Result<MoveOutcome, Miss> {
    let (verb, rest) = input::verb_line(line, actor_id)?;
    let note = queue_verb(world, actor_id, verb, rest)?;
    if gm_seated {
        return Ok(MoveOutcome::Queued(note));
    }
    let advanced = commit_queued(world)?;
    Ok(MoveOutcome::Committed {
        advanced,
        notices: world.take_rolls(),
    })
}

pub fn player_notice_lines(notices: &[RollReport], actor_id: &str) -> String {
    let mut out = String::new();
    for notice in notices {
        if notice.actor_id() != actor_id {
            continue;
        }
        match notice {
            RollReport::Rolled { out: rolled, .. } => {
                out.push_str(&frames::player_check_line(rolled.outcome))
            }
            RollReport::Refused { code, .. } => {
                out.push_str(&frames::player_refused_line(code.token()))
            }
        }
    }
    out
}

pub fn gm_check_frames(notices: &[RollReport]) -> Vec<Vec<u8>> {
    notices
        .iter()
        .filter_map(|notice| match notice {
            RollReport::Rolled { out, .. } => Some(frames::check_event(out)),
            RollReport::Refused { .. } => None,
        })
        .collect()
}

fn queue_verb(
    world: &mut World,
    actor_id: &str,
    verb: &str,
    rest: &str,
) -> Result<QueuedNote, Miss> {
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
    let intent = player_intent(world, actor_id, verb, target.clone())?;
    gate(world.canon(), world.now(), &intent)?;
    let token = match &target {
        Target::Empty => "",
        Target::Actor(id) | Target::Place(id) => id.as_str(),
        Target::Clock(_) => return Err(Reason::Schema.into()),
    };
    let note = QueuedNote {
        id: player_line_id(world, actor_id, &canonical_line(verb, token))?,
        actor_id: actor_id.to_string(),
        move_name: verb.to_string(),
    };
    push_intent(
        world,
        IntentPush {
            id: note.id.clone(),
            actor_id,
            actor_kind: ActorKind::Player,
            move_name: verb,
            target,
            args: IntentArgs::default(),
            rationale_code: String::new(),
            tick_index: None,
        },
    )?;
    Ok(note)
}

pub fn queue_wait(world: &mut World, actor_id: &str) -> Result<QueuedNote, Miss> {
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
    let id = player_line_id(world, actor_id, &input::canonical_line(verb, token))?;
    Ok(Intent {
        id,
        tick_index: world.now().tick_index,
        actor_id: actor_id.to_string(),
        actor_kind: ActorKind::Player,
        source: Source::Player,
        move_name: verb.to_string(),
        target,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    })
}

fn player_line_id(world: &World, actor_id: &str, canonical: &str) -> Result<String, Reason> {
    let tick = world
        .now()
        .tick_index
        .checked_add(1)
        .ok_or(Reason::Schema)?;
    let id = player_intent_id(&world.now().shard_id, tick, actor_id, canonical);
    if world.queued_intents().iter().any(|intent| intent.id == id) {
        return Err(Reason::Schema);
    }
    Ok(id)
}

/// An empty round does not enter `play` and does not advance `world_time`.
pub fn commit_queued(world: &mut World) -> Result<bool, Reason> {
    if world.queued_intents().is_empty() {
        return Ok(false);
    }
    let before = world.now().world_time;
    let wall = wall_now()?;
    commit_moves_at(world, &wall)?;
    Ok(world.now().world_time != before)
}

impl QueuedNote {
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
