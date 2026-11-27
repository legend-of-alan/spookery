//! Whether an actor can pay a cost, paying it, and how long it takes.

use crate::canon::Canon;
use crate::ecs::component::{self, modifier, Component, Meter};
use crate::ecs::{self, change, entity, Presence};
use crate::intent::Intent;
use crate::rules::context::Context;
use crate::rules::expr::{eval, Expr};
use crate::rules::gate;
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cost {
    pub ticks: Expr,
    pub meters: Vec<(String, Expr)>,
    pub items: Vec<(String, Expr)>,
}

impl Cost {
    /// The ticks, the meter amounts, then the item counts.
    pub(crate) fn exprs(&self) -> impl Iterator<Item = &Expr> {
        let amounts = self.meters.iter().chain(&self.items).map(|(_, expr)| expr);
        std::iter::once(&self.ticks).chain(amounts)
    }
}

/// Refuse a cost the actor cannot pay as `CANT_AFFORD`, with the meter or item id as `detail`.
pub(crate) fn afford(context: &Context, cost: &Cost) -> Result<(), gate::Refusal> {
    let (world, intent) = (context.world, context.intent);
    let actor_id = intent
        .actor_id
        .as_ref()
        .ok_or_else(|| gate::Refusal::new(intent, Reason::UnknownId))?;
    let cant_afford = |missing: &str| gate::Refusal {
        id: intent.id.clone(),
        code: Reason::CantAfford,
        detail: Some(missing.to_string()),
    };
    for (key, expr) in &cost.meters {
        let amount = eval::eval(context, expr).map_err(|_| cant_afford(key))?;
        if amount < 0 {
            return Err(cant_afford(key));
        }
        let current =
            modifier::value(world.actors(), actor_id, key).map_err(|_| cant_afford(key))?;
        if current < amount {
            return Err(cant_afford(key));
        }
    }
    for (id, expr) in &cost.items {
        let amount = eval::eval(context, expr).map_err(|_| cant_afford(id))?;
        let count = item_count(amount).map_err(|_| cant_afford(id))?;
        let items = world.actors().get::<component::Item>(actor_id);
        if items.get(id).copied().unwrap_or(0) < count {
            return Err(cant_afford(id));
        }
    }
    Ok(())
}

fn item_count(amount: i64) -> Result<u32, Reason> {
    u32::try_from(amount).map_err(|_| Reason::CantAfford)
}

/// The ticks `cost` takes, read for the intent's actor. A negative count is `OUT_OF_RANGE`.
pub(crate) fn read_ticks(context: &Context, cost: &Cost) -> Result<u64, Reason> {
    u64::try_from(eval::eval(context, &cost.ticks)?).map_err(|_| Reason::OutOfRange)
}

/// Meters and items are paid at the start. Time is a duration for later completion. A meter
/// cost of 0 leaves the meter as it was and still counts against the key budget.
pub(crate) fn pay(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    intent: &Intent,
    cost: &Cost,
) -> Result<(), Reason> {
    let actor_id = intent.actor_id.as_ref().ok_or(Reason::UnknownId)?;
    for (key, expr) in &cost.meters {
        let context = Context::new(canon, world, presence, intent);
        let amount = eval::eval(&context, expr)?;
        if amount < 0 {
            return Err(Reason::CantAfford);
        }
        (world.actors_mut().get_mut::<Meter>(actor_id)).shift(key, -amount)?;
        if amount == 0 {
            world.count_key(&change::Key::new(
                Meter::ID,
                entity::IdRef::Actor(actor_id),
                Some(key),
            ))?;
        }
    }
    for (id, expr) in &cost.items {
        let context = Context::new(canon, world, presence, intent);
        let count = item_count(eval::eval(&context, expr)?)?;
        let taken = -i64::from(count);
        match (world.actors_mut().get_mut::<component::Item>(actor_id)).shift(id, taken, None) {
            Err(Reason::Budget) => return Err(Reason::Budget),
            Err(_) => return Err(Reason::CantAfford),
            Ok(()) => {}
        }
    }
    Ok(())
}
