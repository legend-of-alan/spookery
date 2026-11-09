//! Whether an actor can pay a cost, and paying it.

use crate::draft::Draft;
use crate::intent::Intent;
use crate::rules::context::Context;
use crate::rules::effect;
use crate::rules::expr::{self, Dice};
use crate::rules::gate;
use crate::rules::rule::Cost;
use crate::state::change;
use crate::state::id::EntityId;
use crate::state::meter::MeterWrite;
use crate::Reason;

/// Refuse a cost the actor cannot pay as `CANT_AFFORD`, with the meter or item id as `detail`.
pub(crate) fn afford(context: &Context, cost: &Cost) -> Result<(), gate::Refusal> {
    let (canon, state, intent) = (context.canon, context.state, context.intent);
    let who = intent
        .actor_id()
        .ok_or_else(|| gate::Refusal::new(intent, Reason::UnknownId))?;
    let cant_afford = |missing: &str| gate::Refusal {
        id: intent.id.clone(),
        code: Reason::CantAfford,
        detail: Some(missing.to_string()),
    };
    for (key, expr) in &cost.meters {
        if expr.needs_dice() {
            continue;
        }
        let amount = expr::eval(context, expr).map_err(|_| cant_afford(key))?;
        if amount < 0 {
            return Err(cant_afford(key));
        }
        let current = state
            .meter_value(canon, &EntityId::Actor(who.clone()), key)
            .map_err(|_| cant_afford(key))?;
        if current < amount {
            return Err(cant_afford(key));
        }
    }
    for (id, expr) in &cost.items {
        if expr.needs_dice() {
            continue;
        }
        let amount = expr::eval(context, expr).map_err(|_| cant_afford(id))?;
        let count = item_count(amount).map_err(|_| cant_afford(id))?;
        let held = state
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
    let who = intent.actor_id().ok_or(Reason::UnknownId)?;
    let entity = EntityId::Actor(who.clone());
    for (key, expr) in &cost.meters {
        let context = Context::new(draft.canon(), draft.state(), draft.presence, intent);
        let amount = expr::eval_with_dice(&context, expr, dice)?;
        if amount < 0 {
            return Err(Reason::CantAfford);
        }
        effect::write_meter(draft, &entity, key, MeterWrite::Offset(-amount))?;
        if amount == 0 {
            // A cost of 0 leaves the meter as it was and still takes a key of the budget.
            draft.track(change::Key::ActorMeter(who.to_string(), key.clone()))?;
        }
    }
    for (id, expr) in &cost.items {
        let context = Context::new(draft.canon(), draft.state(), draft.presence, intent);
        let count = item_count(expr::eval_with_dice(&context, expr, dice)?)?;
        // The key budget keeps its own code; any other refusal of the take is a shortfall.
        match effect::shift_item(draft, &entity, id, -i64::from(count), None) {
            Err(Reason::Budget) => return Err(Reason::Budget),
            Err(_) => return Err(Reason::CantAfford),
            Ok(()) => {}
        }
    }
    Ok(())
}
