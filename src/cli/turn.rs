use spookery_kernel::check::CheckNotice;
use spookery_kernel::ids::player_intent_id;
use spookery_kernel::model::{ActorKind, ActorLive, Intent, IntentArgs, Source, Target};
use spookery_kernel::moves::{ActionRow, Audience, MoveToken};
use spookery_kernel::shard::Shard;
use spookery_kernel::Reason;
use spookery_tick::commit_turn_at;

use super::line::{self, canonical_line};
use super::protocol;
use crate::schedule::{self, seconds_since, wall_now};

pub use line::{classify_player_line, PlayerLine};

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

pub fn turn(
    shard: &mut Shard,
    actor_id: &str,
    line: &str,
    gm_seated: bool,
) -> Result<Turn, Reason> {
    let (verb, rest) = line::verb_line(line, actor_id)?;
    let Some(row) = shard
        .canon()
        .moves
        .get(verb)
        .and_then(|def| def.row.clone())
    else {
        return Err(Reason::NotInSpec);
    };
    if row.audience != Some(Audience::Player) {
        return Err(Reason::NotInSpec);
    }
    let note = queue_row(shard, actor_id, verb, rest, &row)?;
    if gm_seated {
        return Ok(Turn::Queued(note));
    }
    let advanced = commit_queued(shard)?;
    Ok(Turn::Committed {
        advanced,
        notices: shard.take_rolls(),
    })
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

/// A named item must already be on the acting sheet. The line is refused before queue.
fn require_item(shard: &Shard, actor_id: &str, row: &ActionRow) -> Result<(), Reason> {
    let Some(item) = row.item.as_deref() else {
        return Ok(());
    };
    let held = shard
        .sheets()
        .get(actor_id)
        .is_some_and(|sheet| sheet.inventory.iter().any(|id| id == item));
    if held {
        Ok(())
    } else {
        Err(Reason::MissingItem)
    }
}

/// The actor or place token is stored as typed. An unknown id stays the tick's existing refusal.
fn queue_row(
    shard: &mut Shard,
    actor_id: &str,
    verb: &str,
    rest: &str,
    row: &ActionRow,
) -> Result<QueuedNote, Reason> {
    if !shard
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
    {
        return Err(Reason::Schema);
    }
    let target = match row.token {
        MoveToken::None => {
            if !rest.is_empty() {
                return Err(Reason::Schema);
            }
            Target::Empty
        }
        MoveToken::Actor => {
            let (named, extra) = line::split_token(rest);
            if named.is_empty() || !extra.is_empty() {
                return Err(Reason::Schema);
            }
            Target::Actor(named.to_string())
        }
        MoveToken::Place => {
            let (place, extra) = line::split_token(rest);
            if place.is_empty() || !extra.is_empty() {
                return Err(Reason::Schema);
            }
            Target::Place(place.to_string())
        }
    };
    require_item(shard, actor_id, row)?;
    let token = match &target {
        Target::Empty => "",
        Target::Actor(id) | Target::Place(id) => id.as_str(),
        Target::Clock(_) | Target::Rumor(_) => return Err(Reason::Schema),
    };
    let note = QueuedNote {
        id: player_line_id(shard, actor_id, &canonical_line(verb, token))?,
        actor_id: actor_id.to_string(),
        move_name: verb.to_string(),
    };
    push_intent(
        shard,
        note.id.clone(),
        actor_id,
        ActorKind::Player,
        verb,
        target,
        IntentArgs::default(),
        String::new(),
        None,
    )?;
    Ok(note)
}

pub fn queue_wait(shard: &mut Shard, actor_id: &str) -> Result<QueuedNote, Reason> {
    let Some(row) = shard
        .canon()
        .moves
        .get("wait")
        .and_then(|def| def.row.clone())
    else {
        return Err(Reason::NotInSpec);
    };
    if row.audience != Some(Audience::Player) {
        return Err(Reason::NotInSpec);
    }
    queue_row(shard, actor_id, "wait", "", &row)
}

fn player_line_id(shard: &Shard, actor_id: &str, canonical: &str) -> Result<String, Reason> {
    let tick = shard
        .now()
        .tick_index
        .checked_add(1)
        .ok_or(Reason::Schema)?;
    let id = player_intent_id(&shard.now().shard_id, tick, actor_id, canonical);
    if shard.queued_intents().iter().any(|intent| intent.id == id) {
        return Err(Reason::Schema);
    }
    Ok(id)
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
