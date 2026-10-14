use crate::canon::Canon;
use crate::error::Reason;
use crate::model::Now;
use crate::rules::expr::{self, Ctx};
use crate::rules::rule::{Cond, Who};

/// Every condition holds. A failed `has_item` is [`Reason::MissingItem`].
pub fn holds(canon: &Canon, now: &Now, ctx: &Ctx<'_>, conds: &[Cond]) -> Result<(), Reason> {
    for cond in conds {
        if !one(canon, now, ctx, cond)? {
            return Err(reason_of(cond));
        }
    }
    Ok(())
}

fn reason_of(cond: &Cond) -> Reason {
    match cond {
        Cond::HasItem { .. } => Reason::MissingItem,
        Cond::All(inner) | Cond::Any(inner) => {
            inner.first().map(reason_of).unwrap_or(Reason::NotInSpec)
        }
        Cond::Not(inner) => reason_of(inner),
        _ => Reason::NotInSpec,
    }
}

fn one(canon: &Canon, now: &Now, ctx: &Ctx<'_>, cond: &Cond) -> Result<bool, Reason> {
    match cond {
        Cond::HasItem { who, id, count } => {
            let actor = expr::actor_of(now, ctx, who)?;
            let held = now
                .actors
                .get(&actor)
                .map(|actor| actor.count(id))
                .unwrap_or(0);
            Ok(held >= *count)
        }
        Cond::Wielding { who, id } => {
            let actor = expr::actor_of(now, ctx, who)?;
            Ok(now
                .actors
                .get(&actor)
                .is_some_and(|actor| actor.wielded() == Some(id.as_str())))
        }
        Cond::Meter {
            who,
            key,
            op,
            value,
        } => {
            let actor = expr::actor_of(now, ctx, who)?;
            let def = canon.meters.get(key).ok_or(Reason::UnknownKey)?;
            let live = now
                .actors
                .get(&actor)
                .ok_or(Reason::UnknownKey)?
                .meters()
                .get(key)
                .ok_or(Reason::UnknownKey)?;
            let current = crate::meter::current(def, live)?;
            let right = expr::eval(canon, now, ctx, value)?;
            Ok(op.holds(current, right))
        }
        Cond::Flag { who, key, is } => {
            if matches!(who, Who::World) {
                return Ok(false);
            }
            let actor = expr::actor_of(now, ctx, who)?;
            let on = now
                .actors
                .get(&actor)
                .is_some_and(|actor| actor.flags().iter().any(|flag| flag == key));
            Ok(on == *is)
        }
        Cond::At { who, place } => {
            let actor = expr::actor_of(now, ctx, who)?;
            Ok(now
                .actors
                .get(&actor)
                .is_some_and(|actor| actor.place() == place))
        }
        Cond::Knows { who, fact } => {
            let actor = expr::actor_of(now, ctx, who)?;
            Ok(now
                .actors
                .get(&actor)
                .is_some_and(|actor| actor.known_facts().iter().any(|id| id == fact)))
        }
        Cond::Clock { id, op, value } => {
            let clock = now.clocks.get(id).ok_or(Reason::UnknownKey)?;
            let right = expr::eval(canon, now, ctx, value)?;
            Ok(!clock.paused && op.holds(clock.value, right))
        }
        Cond::Hour { from, to } => Ok(hour_in(*from, *to, world_hour(now.world_time))),
        Cond::Humans { op, value } => Ok(op.holds(i64::from(now.presence.humans), *value)),
        Cond::NoteExists(id) => Ok(now.messages.iter().any(|message| &message.id == id)),
        Cond::All(inner) => {
            for cond in inner {
                if !one(canon, now, ctx, cond)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Cond::Any(inner) => {
            for cond in inner {
                if one(canon, now, ctx, cond)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Cond::Not(inner) => Ok(!one(canon, now, ctx, inner)?),
    }
}

pub fn world_hour(world_time: u64) -> i64 {
    ((world_time / 60) % 24) as i64
}

pub fn hour_in(start: i64, end: i64, hour: i64) -> bool {
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
        assert!(hour_in(8, 16, 8));
        assert!(hour_in(8, 16, 15));
        assert!(!hour_in(8, 16, 16));
        assert!(!hour_in(8, 16, 7));
        assert!(hour_in(18, 6, 18));
        assert!(hour_in(18, 6, 5));
        assert!(!hour_in(18, 6, 6));
        assert!(!hour_in(18, 6, 12));
        assert!(!hour_in(8, 8, 8));
    }
}
