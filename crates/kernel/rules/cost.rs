use crate::canon::Canon;
use crate::error::Reason;
use crate::key::StateKey;
use crate::meter::{self, MeterWrite};
use crate::model::Now;
use crate::rules::rule::Cost;
use crate::world::Draft;

/// What the actor cannot pay. The token is `CANT_AFFORD`; `what` names the gap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub what: String,
}

pub fn can_pay(canon: &Canon, now: &Now, who: &str, cost: &Cost) -> Result<(), Gap> {
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
            .map(|actor| actor.inventory().iter().filter(|item| *item == id).count())
            .unwrap_or(0);
        if held < usize::try_from(*count).unwrap_or(usize::MAX) {
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
    if !cost.items.is_empty() {
        let inventory = draft
            .now_mut()
            .actors
            .get_mut(who)
            .ok_or(Reason::UnknownKey)?
            .inventory_mut();
        for (id, count) in &cost.items {
            for _ in 0..*count {
                let Some(index) = inventory.iter().position(|item| item == id) else {
                    return Err(Reason::CantAfford);
                };
                inventory.remove(index);
            }
        }
    }
    Ok(())
}

pub fn duration(cost: &Cost) -> u64 {
    cost.minutes
}
