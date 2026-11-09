//! Whether the conditions on a rule hold.

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target, Viewer};
use crate::rules::context::Context;
use crate::rules::expr;
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
        Cond::HasItem { who, id, count } => {
            let actor = expr::resolve_who_id(context, who)?;
            let id = expr::resolve_item_id(context, id)?;
            let held = state
                .actors
                .get(&actor)
                .map(|actor| actor.item_count(&id))
                .unwrap_or(0);
            Ok(held >= *count)
        }
        Cond::Wielding { who, id } => {
            let actor = expr::resolve_who_id(context, who)?;
            let id = expr::resolve_item_id(context, id)?;
            Ok(state
                .actors
                .get(&actor)
                .is_some_and(|actor| actor.wielded.as_deref() == Some(id.as_str())))
        }
        Cond::Meter {
            who,
            key,
            op,
            value,
        } => {
            let entity = expr::resolve_entity(context, who)?;
            let current = state.meter_value(context.canon, &entity, key)?;
            let right = expr::eval(context, value)?;
            Ok(op.holds(current, right))
        }
        Cond::Flag { who, key, is } => {
            let entity = expr::resolve_entity(context, who)?;
            Ok(state.has_flag(&entity, key) == *is)
        }
        Cond::At { who, place } => {
            let actor = expr::resolve_who_id(context, who)?;
            Ok(state
                .actors
                .get(&actor)
                .is_some_and(|actor| actor.place == *place))
        }
        Cond::Knows { who, fact } => {
            let actor = expr::resolve_who_id(context, who)?;
            Ok(state
                .actors
                .get(&actor)
                .is_some_and(|actor| actor.known_facts.iter().any(|id| id == fact)))
        }
        Cond::Cycle { name, from, to } => {
            let length = context.canon.budget.cycles.get(name);
            let tick = state.world_time % length.ok_or(Reason::UnknownId)?;
            Ok(is_in_window(*from, *to, tick))
        }
        Cond::Players { op, value } => Ok(op.holds(i64::from(context.presence.players), *value)),
        Cond::NoteExists(id) => Ok(state.notes.iter().any(|note| &note.id == id)),
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
