//! Queue a player verb until the round commits it.

use spookery_kernel::api::rules::gate::GateRefusal;
pub use spookery_kernel::api::world::QueuedMove;
use spookery_kernel::api::world::World;
use spookery_kernel::api::Reason;
use spookery_round::{advance_while_busy, play, RoundCause, RoundOutput};

use super::frames;
use super::idle_clock::wall_now;

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
    let queued = match world.queue_player_move(actor_id, line) {
        Ok(queued) => queued,
        Err(refusal) => {
            if !gm_seated && refusal.reason() == Reason::Busy {
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

pub fn queue_wait(world: &mut World, actor_id: &str) -> Result<QueuedMove, GateRefusal> {
    world.queue_wait(actor_id)
}

pub fn player_queued_line(queued: &QueuedMove) -> String {
    frames::player_queued_line(&queued.id)
}

pub fn gm_queued_frame(queued: &QueuedMove) -> Vec<u8> {
    frames::queued_event(&queued.id, &queued.actor_id, &queued.move_name)
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
