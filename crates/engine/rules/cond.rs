//! Whether the conditions on a rule hold.

use crate::canon::Canon;
use crate::ecs::component::Note;
use crate::ecs::id::{ActorId, GlobalId};
use crate::ecs::{self, component, Presence};
use crate::intent::{Intent, Source, Target};
use crate::rules::context::Context;
use crate::rules::expr::eval;
use crate::rules::rule::Cond;
use crate::Reason;

/// Every condition holds. A failed `has_item` is [`Reason::MissingItem`].
pub(crate) fn all_hold(context: &Context, conds: &[Cond]) -> Result<(), Reason> {
    for cond in conds {
        if !holds(context, cond)? {
            return Err(refusal_code(cond));
        }
    }
    Ok(())
}

/// Whether `conds` hold for `actor_id` doing `verb` to `target`. `help`, exit locks and an
/// NPC's habit list read an `if` this way: they have no intent, so they read it against a
/// probe with no id.
pub(crate) fn if_holds(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    actor_id: &ActorId,
    verb: &str,
    target: Target,
    conds: &[Cond],
) -> bool {
    let probe = Intent::new(
        String::new(),
        world.round_index,
        Some(actor_id.clone()),
        Source::Player,
        verb,
        target,
    );
    all_hold(&Context::new(canon, world, presence, &probe), conds).is_ok()
}

fn refusal_code(cond: &Cond) -> Reason {
    match cond {
        Cond::HasItem { .. } => Reason::MissingItem,
        Cond::All(inner) | Cond::Any(inner) => {
            inner.first().map(refusal_code).unwrap_or(Reason::NotInSpec)
        }
        Cond::Not(inner) => refusal_code(inner),
        _ => Reason::NotInSpec,
    }
}

fn holds(context: &Context, cond: &Cond) -> Result<bool, Reason> {
    let world = context.world;
    match cond {
        Cond::HasItem { subject, id, count } => {
            let actor = subject.actor_id(context)?;
            let id = context.item_id(id)?;
            let items = world.actors().get::<component::Item>(actor);
            Ok(i64::from(items.get(&id).copied().unwrap_or(0)) >= eval::eval(context, count)?)
        }
        Cond::Wielding { subject, id } => {
            let actor = subject.actor_id(context)?;
            let id = context.item_id(id)?;
            let wielded = world.actors().get::<component::WieldedItem>(actor).get();
            Ok(wielded.is_some_and(|wielded| wielded == id))
        }
        Cond::Meter { of, key, op, value } => {
            let entity = of.entity_id(context)?;
            let current = (world.entities().get::<component::Meter>(&entity)).value(key)?;
            let right = eval::eval(context, value)?;
            Ok(op.holds(current, right))
        }
        Cond::Flag { of, key, is } => {
            let entity = of.entity_id(context)?;
            let flag = world.entities().get::<component::Flag>(&entity);
            Ok(flag.contains(key) == *is)
        }
        Cond::At { subject, place } => {
            let actor = subject.actor_id(context)?;
            let at = world.actors().get::<component::Place>(actor).get();
            Ok(at.is_some_and(|at| at.is_at(place)))
        }
        Cond::Cycle { id, from, to } => {
            let cycles = &context.canon.budget.cycles;
            let length = *cycles.get(id).ok_or(Reason::UnknownId)?;
            let read_tick = |expr| -> Result<u64, Reason> {
                cycle_tick_of(eval::eval(context, expr)?, length).ok_or(Reason::OutOfRange)
            };
            let tick = world.world_time % length;
            Ok(is_in_window(read_tick(from)?, read_tick(to)?, tick))
        }
        Cond::Players { op, value } => {
            let players = i64::from(context.presence.players);
            Ok(op.holds(players, eval::eval(context, value)?))
        }
        Cond::NoteExists(id) => Ok(world.globals().get::<Note>(&GlobalId).contains(id)),
        Cond::All(inner) => {
            for cond in inner {
                if !holds(context, cond)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Cond::Any(inner) => {
            for cond in inner {
                if holds(context, cond)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Cond::Not(inner) => Ok(!holds(context, inner)?),
    }
}

/// `bound` as a tick of a cycle `length` ticks long, when it is not negative and below the
/// length. Load checks a constant bound with it, and play a bound read from a meter.
pub(crate) fn cycle_tick_of(bound: i64, length: u64) -> Option<u64> {
    u64::try_from(bound).ok().filter(|tick| *tick < length)
}

fn is_in_window(start: u64, end: u64, tick: u64) -> bool {
    if start < end {
        start <= tick && tick < end
    } else if start > end {
        tick >= start || tick < end
    } else {
        false
    }
}
