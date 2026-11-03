//! Whether the conditions on a rule hold.

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target, Viewer};
use crate::model::{Presence, State, MINUTES_PER_DAY, MINUTES_PER_HOUR};
use crate::rules::context::Context;
use crate::rules::expr;
use crate::rules::rule::{Cond, Who};
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
        viewer.clone(),
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
            let actor = expr::resolve_who_id(context, who)?;
            let current = state.meter_value(context.canon, &actor, key)?;
            let right = expr::eval(context, value)?;
            Ok(op.holds(current, right))
        }
        Cond::Flag { who, key, is } => {
            if matches!(who, Who::World) {
                let on = state.flags.iter().any(|flag| flag == key);
                return Ok(on == *is);
            }
            let actor = expr::resolve_who_id(context, who)?;
            let on = state
                .actors
                .get(&actor)
                .is_some_and(|actor| actor.flags.iter().any(|flag| flag == key));
            Ok(on == *is)
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
        Cond::Clock { id, op, value } => {
            let clock = state.clocks.get(id).ok_or(Reason::UnknownId)?;
            let right = expr::eval(context, value)?;
            Ok(!clock.paused && op.holds(clock.value, right))
        }
        Cond::Hour { from, to } => Ok(is_hour_in(*from, *to, world_hour(state.world_time))),
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

fn world_hour(world_time: u64) -> i64 {
    ((world_time % MINUTES_PER_DAY) / MINUTES_PER_HOUR) as i64
}

fn is_hour_in(start: i64, end: i64, hour: i64) -> bool {
    if start < end {
        start <= hour && hour < end
    } else if start > end {
        hour >= start || hour < end
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_window_wraps_and_a_point_is_off() {
        assert_eq!(world_hour(1080), 18);
        assert_eq!(world_hour(0), 0);
        assert_eq!(world_hour(1440), 0);
        assert!(is_hour_in(8, 16, 8));
        assert!(is_hour_in(8, 16, 15));
        assert!(!is_hour_in(8, 16, 16));
        assert!(!is_hour_in(8, 16, 7));
        assert!(is_hour_in(18, 6, 18));
        assert!(is_hour_in(18, 6, 5));
        assert!(!is_hour_in(18, 6, 6));
        assert!(!is_hour_in(18, 6, 12));
        assert!(!is_hour_in(8, 8, 8));
    }
}
