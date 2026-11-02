//! Whether an actor can pay a cost, and paying it.

use crate::draft::Draft;
use crate::intent::Intent;
use crate::key::{StateKey, StateValue};
use crate::meter::{self, MeterWrite};
use crate::model;
use crate::rule::Cost;
use crate::rules::case::Case;
use crate::rules::expr::{self, Dice};
use crate::rules::gate;
use crate::Reason;

/// Refuse a cost the actor cannot pay as `CANT_AFFORD`, with the meter or item id as `detail`.
pub(crate) fn afford(case: &Case, cost: &Cost) -> Result<(), gate::Refusal> {
    let (canon, now, intent) = (case.canon, case.now, case.intent);
    let who = intent.actor.actor_id();
    let cant_afford = |missing: &str| gate::Refusal {
        id: intent.id.clone(),
        code: Reason::CantAfford,
        detail: Some(missing.to_string()),
    };
    for (key, expr) in &cost.meters {
        if expr.needs_dice() {
            continue;
        }
        let amount = expr::eval(case, expr).map_err(|_| cant_afford(key))?;
        if amount < 0 {
            return Err(cant_afford(key));
        }
        let Some(def) = canon.meters.get(key) else {
            return Err(cant_afford(key));
        };
        let Some(live) = now
            .actors
            .get(who)
            .and_then(|actor| actor.meters().get(key))
        else {
            return Err(cant_afford(key));
        };
        let current = meter::current(def, live).map_err(|_| cant_afford(key))?;
        if current < amount {
            return Err(cant_afford(key));
        }
    }
    for (id, expr) in &cost.items {
        if expr.needs_dice() {
            continue;
        }
        let amount = expr::eval(case, expr).map_err(|_| cant_afford(id))?;
        let count = item_count(amount).map_err(|_| cant_afford(id))?;
        let held = now
            .actors
            .get(who)
            .map(|actor| actor.item_count(id))
            .unwrap_or(0);
        if held < count {
            return Err(cant_afford(id));
        }
    }
    Ok(())
}

fn item_count(amount: i64) -> Result<u32, Reason> {
    u32::try_from(amount).map_err(|_| Reason::CantAfford)
}

/// Meters and items are paid at the start. Time is a duration for later completion.
pub(crate) fn pay(
    draft: &mut Draft,
    intent: &Intent,
    cost: &Cost,
    dice: &mut dyn Dice,
) -> Result<(), Reason> {
    let who = intent.actor.actor_id();
    for (key, expr) in &cost.meters {
        let case = Case::new(draft.canon(), draft.state(), draft.presence, intent);
        let amount = expr::eval_with_dice(&case, expr, dice)?;
        if amount < 0 {
            return Err(Reason::CantAfford);
        }
        let def = *draft.canon().meters.get(key).ok_or(Reason::UnknownId)?;
        let mut live = draft
            .state()
            .actors
            .get(who)
            .ok_or(Reason::UnknownId)?
            .meters()
            .get(key)
            .cloned()
            .ok_or(Reason::UnknownId)?;
        meter::write(&def, &mut live, MeterWrite::Offset(-amount))?;
        draft.track(StateKey::ActorMeter(who.to_string(), key.clone()))?;
        *draft
            .state_mut()
            .actors
            .get_mut(who)
            .ok_or(Reason::UnknownId)?
            .meters_mut()
            .get_mut(key)
            .ok_or(Reason::UnknownId)? = live;
    }
    for (id, expr) in &cost.items {
        let case = Case::new(draft.canon(), draft.state(), draft.presence, intent);
        let count = item_count(expr::eval_with_dice(&case, expr, dice)?)?;
        let before = i64::from(
            draft
                .state()
                .actors
                .get(who)
                .ok_or(Reason::UnknownId)?
                .item_count(id),
        );
        let changed = {
            let inventory = draft
                .state_mut()
                .actors
                .get_mut(who)
                .ok_or(Reason::UnknownId)?
                .inventory_mut();
            model::shift_count(inventory, id, -i64::from(count), None)
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
        .state()
        .actors
        .get(who)
        .ok_or(Reason::UnknownId)?
        .wielded()
        .map(str::to_string);
    let released = draft
        .state_mut()
        .actors
        .get_mut(who)
        .ok_or(Reason::UnknownId)?
        .release_wielded();
    if released {
        draft.track_before(
            StateKey::ActorWielded(who.to_string()),
            wielded.map(StateValue::Text).unwrap_or(StateValue::Empty),
        )?;
    }
    Ok(())
}
