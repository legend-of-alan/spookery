//! Whether the conditions on a rule hold.

use crate::canon::{Canon, OPEN};
use crate::ecs::component::Note;
use crate::ecs::id::GlobalId;
use crate::ecs::{self, component, Presence};
use crate::intent::{Target, Viewer};
use crate::rules::context::{self, Context};
use crate::rules::expr;
use crate::rules::grammar;
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

/// Whether `conds` hold for `viewer` doing `verb` to `target`. `help`, exit locks and an
/// NPC's habit list read an `if` this way: they have no intent, so they read it against
/// [`context::probe`].
pub(crate) fn if_holds(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    viewer: &Viewer,
    verb: &str,
    target: Target,
    conds: &[Cond],
) -> bool {
    let probe = context::probe(world, viewer, verb, target);
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
            let actor = expr::resolve_actor(context, subject)?;
            let id = expr::resolve_item_id(context, id)?;
            let held = world.actors().get::<component::Item>(actor);
            Ok(i64::from(held.get(&id).copied().unwrap_or(0)) >= expr::eval(context, count)?)
        }
        Cond::Wielding { subject, id } => {
            let actor = expr::resolve_actor(context, subject)?;
            let id = expr::resolve_item_id(context, id)?;
            let wielded = world.actors().get::<component::WieldedItem>(actor).get();
            Ok(wielded.is_some_and(|wielded| wielded == id))
        }
        Cond::Meter { of, key, op, value } => {
            let entity = expr::resolve_entity(context, of)?;
            let current = (world.entities().get::<component::Meter>(&entity)).value(key)?;
            let right = expr::eval(context, value)?;
            Ok(op.holds(current, right))
        }
        Cond::Flag {
            of: grammar::Object::Exit(exit),
            key,
            is,
        } => Ok((key == OPEN && world.places().is_exit_open(exit)) == *is),
        Cond::Flag { of, key, is } => {
            let entity = expr::resolve_entity(context, of)?;
            let flag = world.entities().get::<component::Flag>(&entity);
            Ok(flag.contains(key) == *is)
        }
        Cond::At { subject, place } => {
            let actor = expr::resolve_actor(context, subject)?;
            let at = world.actors().get::<component::Place>(actor).get();
            Ok(at.is_some_and(|at| at == place))
        }
        Cond::Knows { subject, fact } => {
            let actor = expr::resolve_actor(context, subject)?;
            let known = world.actors().get::<component::KnownFacts>(actor).get();
            Ok(known.is_some_and(|facts| facts.iter().any(|id| id == fact)))
        }
        Cond::Cycle { id, from, to } => {
            let cycles = &context.canon.budget.cycles;
            let length = *cycles.get(id).ok_or(Reason::UnknownId)?;
            // A bound is a tick of the cycle: not negative and below its length.
            let bound = |expr| -> Result<u64, Reason> {
                let tick = u64::try_from(expr::eval(context, expr)?).ok();
                tick.filter(|tick| *tick < length).ok_or(Reason::OutOfRange)
            };
            let tick = world.world_time % length;
            Ok(is_in_window(bound(from)?, bound(to)?, tick))
        }
        Cond::Players { op, value } => {
            let players = i64::from(context.presence.players);
            Ok(op.holds(players, expr::eval(context, value)?))
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

fn is_in_window(start: u64, end: u64, tick: u64) -> bool {
    if start < end {
        start <= tick && tick < end
    } else if start > end {
        tick >= start || tick < end
    } else {
        false
    }
}
