//! Whether an actor can pay a cost, and paying it.

use crate::draft::Draft;
use crate::intent::Intent;
use crate::model::key::StateKey;
use crate::model::meter::MeterWrite;
use crate::rules::case::Case;
use crate::rules::effect;
use crate::rules::expr::{self, Dice};
use crate::rules::gate;
use crate::rules::rule::Cost;
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
        let current = now
            .meter_value(canon, who, key)
            .map_err(|_| cant_afford(key))?;
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
        effect::write_meter(draft, who, key, MeterWrite::Offset(-amount))?;
        if amount == 0 {
            // A cost of 0 leaves the meter as it was and still takes a key of the budget.
            draft.track(StateKey::ActorMeter(who.to_string(), key.clone()))?;
        }
    }
    let holder = effect::Holder::Actor(who.to_string());
    for (id, expr) in &cost.items {
        let case = Case::new(draft.canon(), draft.state(), draft.presence, intent);
        let count = item_count(expr::eval_with_dice(&case, expr, dice)?)?;
        // The key budget keeps its own code; any other refusal of the take is a shortfall.
        match effect::shift_item(draft, &holder, id, -i64::from(count), None) {
            Err(Reason::Budget) => return Err(Reason::Budget),
            Err(_) => return Err(Reason::CantAfford),
            Ok(()) => {}
        }
    }
    Ok(())
}
