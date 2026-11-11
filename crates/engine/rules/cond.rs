//! Whether the conditions on a rule hold.

use crate::canon::{Canon, OPEN};
use crate::intent::{Intent, Source, Target, Viewer};
use crate::rules::context::Context;
use crate::rules::expr;
use crate::rules::grammar;
use crate::rules::rule::Cond;
use crate::state::{Presence, State};
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
/// NPC's habit list read an `if` this way: they have no intent, so they read it against a
/// probe with no id.
pub(crate) fn if_holds(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    viewer: &Viewer,
    verb: &str,
    target: Target,
    conds: &[Cond],
) -> bool {
    let probe = Intent::new(
        String::new(),
        state.round_index,
        Some(viewer.clone()),
        Source::Player,
        verb,
        target,
    );
    all_hold(&Context::new(canon, state, presence, &probe), conds).is_ok()
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
    let state = context.state;
    match cond {
        Cond::HasItem { of, id, count } => {
            let actor = expr::resolve_actor(context, of)?;
            let id = expr::resolve_item_id(context, id)?;
            let held = state
                .actors()
                .items
                .get(&(actor.clone(), id))
                .copied()
                .unwrap_or(0);
            Ok(held >= *count)
        }
        Cond::Wielding { of, id } => {
            let actor = expr::resolve_actor(context, of)?;
            let id = expr::resolve_item_id(context, id)?;
            Ok(state
                .actors()
                .wielded
                .get(actor)
                .is_some_and(|wielded| *wielded == id))
        }
        Cond::Meter { of, key, op, value } => {
            let entity = expr::resolve_entity(context, of)?;
            let current = state.entity(&entity).meter_value(context.canon, key)?;
            let right = expr::eval(context, value)?;
            Ok(op.holds(current, right))
        }
        Cond::Flag {
            of: grammar::Object::Exit(exit),
            key,
            is,
        } => Ok((key == OPEN && state.places().is_exit_open(exit)) == *is),
        Cond::Flag { of, key, is } => {
            let entity = expr::resolve_entity(context, of)?;
            Ok(state.entity(&entity).has_flag(key) == *is)
        }
        Cond::At { of, place } => {
            let actor = expr::resolve_actor(context, of)?;
            Ok(state
                .actors()
                .place
                .get(actor)
                .is_some_and(|at| at == place))
        }
        Cond::Knows { of, fact } => {
            let actor = expr::resolve_actor(context, of)?;
            Ok(state
                .actors()
                .known_facts
                .get(actor)
                .is_some_and(|facts| facts.iter().any(|id| id == fact)))
        }
        Cond::Cycle { name, from, to } => {
            let length = context.canon.budget.cycles.get(name);
            let tick = state.world_time % length.ok_or(Reason::UnknownId)?;
            Ok(is_in_window(*from, *to, tick))
        }
        Cond::Players { op, value } => Ok(op.holds(i64::from(context.presence.players), *value)),
        Cond::NoteExists(id) => Ok(state.notes().iter().any(|note| &note.id == id)),
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
