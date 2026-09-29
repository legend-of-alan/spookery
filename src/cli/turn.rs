use spookery_kernel::check::CheckNotice;
use spookery_kernel::model::{ActorKind, ActorLive, Intent, IntentArgs, Source, Target};
use spookery_kernel::store::Shard;
use spookery_kernel::Reason;
use spookery_tick::commit_turn_at;

use super::protocol;
use crate::schedule::{self, seconds_since, wall_now};

pub struct QueuedNote {
    pub id: String,
    pub actor_id: String,
    pub move_name: String,
}

pub enum Turn {
    Queued(QueuedNote),
    Committed {
        advanced: bool,
        notices: Vec<CheckNotice>,
    },
}

/// `say` and `talk` are speech. `wait`, `attempt`, and `travel` take the queue.
/// Any other verb is `NOT_IN_SPEC` here. Speech's `NOT_IN_SPEC` is not the signal.
pub enum PlayerLine {
    Speech,
    Turn,
}

pub fn classify_player_line(line: &str, seated: &str) -> Result<PlayerLine, Reason> {
    let (verb, _) = verb_line(line, seated)?;
    if verb.is_empty() {
        return Err(Reason::Schema);
    }
    match verb {
        "say" | "talk" => Ok(PlayerLine::Speech),
        "wait" | "attempt" | "travel" => Ok(PlayerLine::Turn),
        _ => Err(Reason::NotInSpec),
    }
}

pub fn turn(
    shard: &mut Shard,
    actor_id: &str,
    line: &str,
    gm_seated: bool,
) -> Result<Turn, Reason> {
    let (verb, rest) = verb_line(line, actor_id)?;
    match verb {
        "attempt" => {
            let note = queue_attempt(shard, actor_id, rest)?;
            if gm_seated {
                return Ok(Turn::Queued(note));
            }
            let advanced = commit_queued(shard)?;
            Ok(Turn::Committed {
                advanced,
                notices: shard.take_rolls(),
            })
        }
        "wait" if rest.is_empty() => {
            let note = queue_wait(shard, actor_id)?;
            if gm_seated {
                return Ok(Turn::Queued(note));
            }
            let advanced = commit_queued(shard)?;
            Ok(Turn::Committed {
                advanced,
                notices: shard.take_rolls(),
            })
        }
        "wait" => Err(Reason::Schema),
        "travel" => {
            let note = queue_travel(shard, actor_id, rest)?;
            if gm_seated {
                return Ok(Turn::Queued(note));
            }
            let advanced = commit_queued(shard)?;
            Ok(Turn::Committed {
                advanced,
                notices: shard.take_rolls(),
            })
        }
        _ => Err(Reason::NotInSpec),
    }
}

/// Seat check and interval check. A tick that runs borrows this `Shard`.
/// Returns whether `world_time` advanced.
pub fn on_scheduled_wake(
    shard: &mut Shard,
    player_held: bool,
    gm_held: bool,
) -> Result<bool, Reason> {
    if player_held || gm_held {
        return Ok(false);
    }
    let every = shard.budget().tick_every_real_seconds;
    if seconds_since(&shard.now().last_tick_at)? < every {
        return Ok(false);
    }
    let before = shard.now().world_time;
    schedule::wake_shard(shard)?;
    Ok(shard.now().world_time != before)
}

pub fn queue_attempt(shard: &mut Shard, actor_id: &str, rest: &str) -> Result<QueuedNote, Reason> {
    if shard.now().presence.humans == 0 {
        return Err(Reason::LiveOnly);
    }
    if !shard
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
    {
        return Err(Reason::Schema);
    }
    let (check, rest) = split_token(rest);
    if check.is_empty() || rest.is_empty() {
        return Err(Reason::Schema);
    }
    let (target_token, extra) = split_token(rest);
    if target_token.is_empty() {
        return Err(Reason::Schema);
    }
    if !extra.is_empty() {
        return Err(Reason::NotInSpec);
    }
    let Some(target) = integer_token(target_token) else {
        return Err(Reason::NotInSpec);
    };
    let note = QueuedNote {
        id: mint_intent_id()?,
        actor_id: actor_id.to_string(),
        move_name: "attempt".to_string(),
    };
    push_intent(
        shard,
        note.id.clone(),
        actor_id,
        ActorKind::Player,
        "attempt",
        Target::Empty,
        IntentArgs {
            check: Some(check.to_string()),
            check_target: Some(target),
            ..IntentArgs::default()
        },
        String::new(),
        None,
    )?;
    Ok(note)
}

pub fn player_notice_lines(notices: &[CheckNotice], actor_id: &str) -> String {
    let mut out = String::new();
    for notice in notices {
        if notice.actor_id() != actor_id {
            continue;
        }
        match notice {
            CheckNotice::Rolled { out: rolled, .. } => {
                out.push_str(&protocol::player_check_line(rolled.success))
            }
            CheckNotice::Refused { code, .. } => {
                out.push_str(&protocol::player_refused_line(code.token()))
            }
        }
    }
    out
}

pub fn gm_check_frames(notices: &[CheckNotice]) -> Vec<Vec<u8>> {
    notices
        .iter()
        .filter_map(|notice| match notice {
            CheckNotice::Rolled { out, .. } => Some(protocol::check_event(out)),
            CheckNotice::Refused { .. } => None,
        })
        .collect()
}

fn integer_token(token: &str) -> Option<i64> {
    let digits = token.strip_prefix('-').unwrap_or(token);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

/// One place id. A place the pack does not have is that tick's kernel refusal.
fn queue_travel(shard: &mut Shard, actor_id: &str, rest: &str) -> Result<QueuedNote, Reason> {
    if !shard
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
    {
        return Err(Reason::Schema);
    }
    let (place, extra) = split_token(rest);
    if place.is_empty() || !extra.is_empty() {
        return Err(Reason::Schema);
    }
    let note = QueuedNote {
        id: mint_intent_id()?,
        actor_id: actor_id.to_string(),
        move_name: "travel".to_string(),
    };
    push_intent(
        shard,
        note.id.clone(),
        actor_id,
        ActorKind::Player,
        "travel",
        Target::Place(place.to_string()),
        IntentArgs::default(),
        String::new(),
        None,
    )?;
    Ok(note)
}

pub fn queue_wait(shard: &mut Shard, actor_id: &str) -> Result<QueuedNote, Reason> {
    if !shard
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
    {
        return Err(Reason::Schema);
    }
    let note = QueuedNote {
        id: mint_intent_id()?,
        actor_id: actor_id.to_string(),
        move_name: "wait".to_string(),
    };
    push_intent(
        shard,
        note.id.clone(),
        actor_id,
        ActorKind::Player,
        "wait",
        Target::Empty,
        IntentArgs::default(),
        String::new(),
        None,
    )?;
    Ok(note)
}

/// An empty round does not enter `run_tick` and does not advance `world_time`.
pub fn commit_queued(shard: &mut Shard) -> Result<bool, Reason> {
    if shard.queued_intents().is_empty() {
        return Ok(false);
    }
    let before = shard.now().world_time;
    let wall = wall_now()?;
    commit_turn_at(shard, &wall)?;
    Ok(shard.now().world_time != before)
}

impl QueuedNote {
    pub fn player_line(&self) -> String {
        protocol::player_queued_line(&self.id)
    }

    pub fn gm_frame(&self) -> Vec<u8> {
        protocol::queued_event(&self.id, &self.actor_id, &self.move_name)
    }
}

pub(super) fn push_intent(
    shard: &mut Shard,
    id: String,
    actor_id: &str,
    actor_kind: ActorKind,
    move_name: &str,
    target: Target,
    args: IntentArgs,
    rationale_code: String,
    tick_index: Option<u64>,
) -> Result<(), Reason> {
    let tick_index = match tick_index {
        Some(tick_index) => tick_index,
        None => shard
            .now()
            .tick_index
            .checked_add(1)
            .ok_or(Reason::Schema)?,
    };
    shard.queue_intent(Intent {
        id,
        tick_index,
        actor_id: actor_id.to_string(),
        actor_kind,
        source: Source::Player,
        move_name: move_name.to_string(),
        target,
        args,
        rationale_code,
    });
    Ok(())
}

fn verb_line<'a>(line: &'a str, seated: &str) -> Result<(&'a str, &'a str), Reason> {
    if line.is_empty() {
        return Err(Reason::Schema);
    }
    let (first, rest) = split_token(line);
    if let Some(id) = first.strip_prefix("player:") {
        if id.is_empty() || id != seated || rest.is_empty() {
            return Err(Reason::Schema);
        }
        return Ok(split_token(rest));
    }
    Ok((first, rest))
}

fn split_token(line: &str) -> (&str, &str) {
    match line.split_once(' ') {
        Some((token, rest)) => (token, rest),
        None => (line, ""),
    }
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
