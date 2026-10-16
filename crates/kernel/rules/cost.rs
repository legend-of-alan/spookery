use crate::canon::Canon;
use crate::draft::Draft;
use crate::error::Reason;
use crate::key::{StateKey, StateValue};
use crate::meter::{self, MeterWrite};
use crate::model::{self, Now};
use crate::rules::rule::Cost;

/// What the actor cannot pay. The token is `CANT_AFFORD`; `what` names the gap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub what: String,
}

pub(crate) fn can_pay(canon: &Canon, now: &Now, who: &str, cost: &Cost) -> Result<(), Gap> {
    for (key, amount) in &cost.meters {
        if *amount < 0 {
            return Err(Gap { what: key.clone() });
        }
        let Some(def) = canon.meters.get(key) else {
            return Err(Gap { what: key.clone() });
        };
        let Some(live) = now
            .actors
            .get(who)
            .and_then(|actor| actor.meters().get(key))
        else {
            return Err(Gap { what: key.clone() });
        };
        let current = meter::current(def, live).map_err(|_| Gap { what: key.clone() })?;
        if current < *amount {
            return Err(Gap { what: key.clone() });
        }
    }
    for (id, count) in &cost.items {
        let held = now
            .actors
            .get(who)
            .map(|actor| actor.count(id))
            .unwrap_or(0);
        if held < *count {
            return Err(Gap { what: id.clone() });
        }
    }
    Ok(())
}

/// Meters and items are paid at the start. Time is a duration for later completion.
pub fn pay(draft: &mut Draft, who: &str, cost: &Cost) -> Result<(), Reason> {
    for (key, amount) in &cost.meters {
        let def = *draft.canon().meters.get(key).ok_or(Reason::UnknownKey)?;
        let mut live = draft
            .now()
            .actors
            .get(who)
            .ok_or(Reason::UnknownKey)?
            .meters()
            .get(key)
            .cloned()
            .ok_or(Reason::UnknownKey)?;
        meter::write(&def, &mut live, MeterWrite::Offset(-amount))?;
        draft.track(StateKey::ActorMeter(who.to_string(), key.clone()))?;
        *draft
            .now_mut()
            .actors
            .get_mut(who)
            .ok_or(Reason::UnknownKey)?
            .meters_mut()
            .get_mut(key)
            .ok_or(Reason::UnknownKey)? = live;
    }
    for (id, count) in &cost.items {
        let before = i64::from(
            draft
                .now()
                .actors
                .get(who)
                .ok_or(Reason::UnknownKey)?
                .count(id),
        );
        let changed = {
            let inventory = draft
                .now_mut()
                .actors
                .get_mut(who)
                .ok_or(Reason::UnknownKey)?
                .inventory_mut();
            model::shift_count(inventory, id, -i64::from(*count), None)
                .map_err(|_| Reason::CantAfford)?
        };
        if changed {
            draft.track_before(
                StateKey::ActorItem(who.to_string(), id.clone()),
                StateValue::Number(before),
            )?;
        }
    }
    let wielded = draft
        .now()
        .actors
        .get(who)
        .ok_or(Reason::UnknownKey)?
        .wielded()
        .map(str::to_string);
    let released = draft
        .now_mut()
        .actors
        .get_mut(who)
        .ok_or(Reason::UnknownKey)?
        .release_wielded();
    if released {
        draft.track_before(
            StateKey::ActorWielded(who.to_string()),
            wielded.map(StateValue::Text).unwrap_or(StateValue::Empty),
        )?;
    }
    Ok(())
}

pub fn duration(cost: &Cost) -> u64 {
    cost.minutes
}
