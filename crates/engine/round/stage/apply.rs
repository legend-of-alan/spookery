//! Start each action: gate it again, pay for it, and finish it now or leave it pending.

use crate::canon::map::Exit;
use crate::canon::Canon;
use crate::ecs::{self, component, PendingAction, Presence};
use crate::intent::Intent;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::effect::{Effect, EffectKind};
use crate::rules::expand::{self, Cause, Expansion};
use crate::rules::gate;
use crate::rules::roll::MoveReport;
use crate::rules::rule::Rule;
use crate::rules::travel;
use crate::store::records::Step;
use crate::Reason;

use crate::round::stage::pick::{Candidate, Resolution};
use crate::round::stage::triggers;
use crate::round::{self, Ledger};

/// What became of one rule's effects in `land`.
pub(crate) enum Landing {
    Landed,
    Refused,
    /// Refused `BUDGET`: the round's state keys are spent.
    OverBudget,
}

pub(crate) fn run(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    candidates: Vec<Candidate>,
    clock: &round::Time,
    dice: &mut round::Dice,
    ledger: &mut Ledger,
) {
    let max_npcs = canon.budget.max_npcs_acting;
    let mut slots = 0u64;
    let mut keys_spent = false;
    for Candidate { intent, resolution } in candidates {
        let intent = &intent;
        let is_pc = |id| world.actors().get::<component::ControlType>(id).is_pc();
        let spends_slot = intent.actor_id.as_ref().is_some_and(|id| !is_pc(id));
        if keys_spent || (spends_slot && slots >= max_npcs) {
            let refusal = gate::Refusal::new(intent, Reason::Budget);
            refuse(world, ledger, intent, refusal);
            continue;
        }
        let point = world.savepoint();
        let landing = match resolution {
            Resolution::Ready { rule } => {
                let context = Context::new(canon, world, presence, intent);
                if let Err(refusal) = gate::admit(&context, rule, None) {
                    refuse(world, ledger, intent, refusal);
                    continue;
                }
                let at = clock.world_time_before;
                start(canon, world, presence, intent, rule, at, dice, ledger)
            }
            Resolution::Cancel => cancel(canon, world, presence, intent, ledger),
            Resolution::Refused(refusal) => {
                refuse(world, ledger, intent, refusal);
                continue;
            }
        };
        match landing {
            Landing::Landed if spends_slot => slots += 1,
            Landing::Landed => {}
            Landing::Refused => world.rollback(point),
            Landing::OverBudget => {
                world.rollback(point);
                keys_spent = true;
            }
        }
        triggers::run(canon, world, presence, dice, ledger);
    }
}

/// Start an action at tick `at`: pay the start cost and the exit's, and count the act when the
/// rule asks. An action that takes no time finishes now. One that takes time is left pending
/// until its ticks pass, and a walk puts the actor on the exit, in neither place.
#[allow(clippy::too_many_arguments)]
fn start(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
    at: u64,
    dice: &mut round::Dice,
    ledger: &mut Ledger,
) -> Landing {
    let context = Context::new(canon, world, presence, intent);
    let exit = travel::exit_of(&context, rule).ok().flatten();
    let ticks = match travel::span(&context, rule) {
        Ok(ticks) => ticks,
        Err(code) => {
            refuse(world, ledger, intent, gate::Refusal::new(intent, code));
            return landing_of(code);
        }
    };
    let mut costs = rule
        .start
        .cost
        .iter()
        .chain(exit.map(|exit| &exit.path_def.cost));
    let mut started = costs.try_for_each(|cost| cost::pay(canon, world, presence, intent, cost));
    if rule.start.counts_as_act {
        started = started.and_then(|()| count_act(world, presence, intent));
    }
    if ticks > 0 {
        let until = at + ticks;
        started =
            started.and_then(|()| pend(canon, world, presence, intent, rule, exit, until, ledger));
    }
    if let Err(code) = started {
        refuse(world, ledger, intent, gate::Refusal::new(intent, code));
        return landing_of(code);
    }
    if ticks > 0 {
        ledger.extra_steps.push(Step::Rule(rule.id.clone()));
        return Landing::Landed;
    }
    finish(canon, world, presence, intent, rule, at, dice, ledger)
}

/// Drop the actor's action under way, with no refund, if the gate still allows it now. Its
/// finish never runs.
fn cancel(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    intent: &Intent,
    ledger: &mut Ledger,
) -> Landing {
    let context = Context::new(canon, world, presence, intent);
    if let Err(refusal) = gate::admit_cancel(&context) {
        refuse(world, ledger, intent, refusal);
        return Landing::Refused;
    }
    let actor_id = intent.actor_id.as_ref().ok_or(Reason::UnknownId);
    match actor_id.and_then(|id| (world.actors_mut().get_mut::<component::Act>(id)).take_pending())
    {
        Ok(_) => Landing::Landed,
        Err(code) => {
            refuse(world, ledger, intent, gate::Refusal::new(intent, code));
            landing_of(code)
        }
    }
}

/// Leave `rule` pending until tick `until`. A walk first puts the actor on its path.
#[allow(clippy::too_many_arguments)]
fn pend(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
    exit: Option<Exit<'_>>,
    until: u64,
    ledger: &mut Ledger,
) -> Result<(), Reason> {
    let actor_id = intent.actor_id.as_ref().ok_or(Reason::UnknownId)?;
    if let Some(exit) = exit {
        let from = world.actors().get::<component::Place>(actor_id).get();
        let kind = EffectKind::Move {
            actor_id: actor_id.clone(),
            from: from.ok_or(Reason::UnknownId)?.clone(),
            to: exit.walker_location(),
            arrived_at: None,
        };
        let effect = Effect::new(intent.actor_id.as_ref(), &intent.id, kind);
        effect.clone().apply(canon, world, presence, intent)?;
        ledger.effects.push(effect);
    }
    let pending = PendingAction {
        verb: intent.verb_id.clone(),
        source: intent.source,
        target: intent.target.clone(),
        rule: rule.id.clone(),
    };
    (world.actors_mut().get_mut::<component::Act>(actor_id)).set_busy(until, Some(pending))
}

/// Finish an action at tick `at`: pay the finish cost, count the act when the rule asks, keep
/// the actor busy through the rule's cooldown, then land every effect the rule expands to, a
/// walk's arrival stamped `at`. A refusal is journaled and the round goes on.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
    at: u64,
    dice: &mut round::Dice,
    ledger: &mut Ledger,
) -> Landing {
    let context = Context::new(canon, world, presence, intent);
    let roller = dice.roller(ledger, &rule.id, intent.actor_id.as_ref());
    let mut expansion = match expand::expand(&context, rule, Cause::Finish, roller) {
        Ok(expansion) => expansion,
        Err(code) => {
            refuse(world, ledger, intent, gate::Refusal::new(intent, code));
            return landing_of(code);
        }
    };
    for effect in &mut expansion.effects {
        if let EffectKind::Move { arrived_at, .. } = &mut effect.kind {
            *arrived_at = Some(at);
        }
    }
    let point = world.savepoint();
    let mut finished = match &rule.finish.cost {
        Some(cost) => cost::pay(canon, world, presence, intent, cost),
        None => Ok(()),
    };
    if rule.finish.counts_as_act {
        finished = finished.and_then(|()| count_act(world, presence, intent));
    }
    if rule.cooldown > 0 {
        finished = finished.and_then(|()| {
            let actor_id = intent.actor_id.as_ref().ok_or(Reason::UnknownId)?;
            let mut actors = world.actors_mut();
            (actors.get_mut::<component::Act>(actor_id)).set_busy(at + rule.cooldown, None)
        });
    }
    if let Err(code) = finished {
        world.rollback(point);
        refuse(world, ledger, intent, gate::Refusal::new(intent, code));
        return landing_of(code);
    }
    let landing = land(canon, world, presence, intent, rule, expansion, ledger);
    if !matches!(landing, Landing::Landed) {
        world.rollback(point);
    }
    landing
}

fn count_act(world: &mut ecs::World, presence: &Presence, intent: &Intent) -> Result<(), Reason> {
    let actor_id = intent.actor_id.as_ref().ok_or(Reason::UnknownId)?;
    let round_index = world.round_index;
    let mut actors = world.actors_mut();
    (actors.get_mut::<component::Act>(actor_id)).count(round_index, presence.players)
}

/// A refusal over the key budget spends the round's keys; any other refuses only this intent.
fn landing_of(code: Reason) -> Landing {
    if code == Reason::Budget {
        Landing::OverBudget
    } else {
        Landing::Refused
    }
}

/// The one landing for a rule's effects: an action's finish or a change rule. Under one
/// savepoint it writes each effect. A failed write rolls back and refuses the intent;
/// otherwise the effects and the rolls are kept and the rule takes its `Rule` step. The
/// expansion's `Cause` decides the steps and who hears a refusal.
pub(crate) fn land(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
    expansion: Expansion,
    ledger: &mut Ledger,
) -> Landing {
    let cause = expansion.cause;
    let point = world.savepoint();
    let written = expansion.effects.iter().try_for_each(|effect| {
        effect.clone().apply(canon, world, presence, intent)?;
        match cause {
            Cause::Finish => ledger.extra_steps.push(Step::EffectApplied),
            Cause::Trigger => {}
        }
        Ok(())
    });
    if let Err(code) = written {
        world.rollback(point);
        let refusal = gate::Refusal::new(intent, code);
        match cause {
            Cause::Finish => refuse(world, ledger, intent, refusal),
            Cause::Trigger => ledger.refused.push(refusal),
        }
        return landing_of(code);
    }
    ledger.effects.extend(expansion.effects);
    ledger.report_roll(expansion.rolled, intent.actor_id.as_ref());
    ledger.reports.extend(expansion.reports);
    ledger.extra_steps.push(Step::Rule(rule.id.clone()));
    Landing::Landed
}

/// Journal the refusal, and tell its code and detail to the actor when that is a player.
pub(crate) fn refuse(
    world: &ecs::World,
    ledger: &mut Ledger,
    intent: &Intent,
    refusal: gate::Refusal,
) {
    let actor_id = intent.actor_id.as_ref();
    if let Some(actor_id) =
        actor_id.filter(|id| world.actors().get::<component::ControlType>(id).is_pc())
    {
        ledger.reports.push(MoveReport::Refused {
            actor_id: actor_id.clone(),
            refusal: refusal.clone(),
        });
    }
    ledger.refused.push(refusal);
}
