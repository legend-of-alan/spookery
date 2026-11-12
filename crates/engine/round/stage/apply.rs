//! Start each action: gate it again, pay for it, and finish it now or leave it pending.

use crate::canon::{Canon, Exit};
use crate::intent::{Intent, Viewer};
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::effect::{Effect, EffectKind};
use crate::rules::expand::{self, Cause, Expansion};
use crate::rules::gate;
use crate::rules::roll::{MoveReport, Roll};
use crate::rules::rule::Rule;
use crate::rules::travel;
use crate::state::{PendingAction, Presence, State};
use crate::store::records::Step;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;
use crate::round::stage::advance::RoundTime;
use crate::round::stage::resolve::{Candidate, Resolution};
use crate::round::stage::triggers;

/// What became of one rule's effects in `land`.
pub(crate) enum Landing {
    Landed,
    Refused,
    /// Refused `BUDGET`: the round's state keys are spent.
    OverBudget,
}

pub(crate) fn run(
    canon: &Canon,
    state: &mut State,
    presence: &Presence,
    candidates: Vec<Candidate>,
    clock: &RoundTime,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) {
    let max_npcs = canon.budget.max_npcs_acting;
    let mut slots = 0u64;
    // Set once a landing goes over the key budget: every later intent is refused `BUDGET`.
    let mut keys_spent = false;
    for Candidate { intent, resolution } in candidates {
        let spends_slot = matches!(intent.actor, Some(Viewer::Npc(_)));
        if keys_spent || (spends_slot && slots >= max_npcs) {
            let refusal = gate::Refusal::new(intent, Reason::Budget);
            refuse(log, intent, refusal, false);
            continue;
        }
        let point = state.savepoint();
        let landing = match resolution {
            Resolution::Ready { rule, rolled } => {
                // The world may have changed since the queue, so the gate checks the rule again.
                let context = Context::new(canon, state, presence, intent);
                if let Err(refusal) = gate::admit(&context, rule, None) {
                    refuse(log, intent, refusal, false);
                    continue;
                }
                let at = clock.world_time_before;
                start(canon, state, presence, intent, rule, rolled, at, dice, log)
            }
            Resolution::Cancel => cancel(canon, state, presence, intent, log),
            Resolution::Refused(refusal) => {
                refuse(log, intent, refusal, false);
                continue;
            }
            Resolution::CannotRoll(code) => {
                refuse(log, intent, gate::Refusal::new(intent, code), true);
                continue;
            }
        };
        match landing {
            Landing::Landed if spends_slot => slots += 1,
            Landing::Landed => {}
            // The costs go back with the effects.
            Landing::Refused => state.rollback(point),
            Landing::OverBudget => {
                state.rollback(point);
                keys_spent = true;
            }
        }
        triggers::run(canon, state, presence, dice, log);
    }
}

/// Start an action at tick `at`: pay the start cost and the exit's, and count the act when the
/// rule asks. An action that takes no time finishes now. One that takes time is left pending
/// until its ticks pass, and a walk puts the actor on the exit, in neither place.
#[allow(clippy::too_many_arguments)]
fn start(
    canon: &Canon,
    state: &mut State,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
    rolled: Option<Roll>,
    at: u64,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Landing {
    let context = Context::new(canon, state, presence, intent);
    let exit = travel::exit_of(&context, rule).ok().flatten().cloned();
    let ticks = travel::span(&context, rule);
    let mut roller = dice.roller(log, &rule.id, intent.actor_id());
    let mut costs = rule
        .start
        .cost
        .iter()
        .chain(exit.as_ref().map(|exit| &exit.cost));
    let mut started =
        costs.try_for_each(|cost| cost::pay(canon, state, presence, intent, cost, &mut roller));
    drop(roller);
    if rule.start.counts_as_act {
        started = started.and_then(|()| count_act(state, presence, intent));
    }
    if ticks > 0 {
        let until = at + ticks;
        started = started.and_then(|()| {
            pend(
                canon,
                state,
                presence,
                intent,
                rule,
                exit.as_ref(),
                until,
                log,
            )
        });
    }
    if let Err(code) = started {
        refuse(log, intent, gate::Refusal::new(intent, code), false);
        return landing_of(code);
    }
    if ticks > 0 {
        log.extra_steps.push(Step::Rule(rule.id.clone()));
        return Landing::Landed;
    }
    finish(canon, state, presence, intent, rule, rolled, at, dice, log)
}

/// Drop the actor's action under way, with no refund, if the gate still allows it now. Its
/// finish never runs.
fn cancel(
    canon: &Canon,
    state: &mut State,
    presence: &Presence,
    intent: &Intent,
    log: &mut RoundLog,
) -> Landing {
    let context = Context::new(canon, state, presence, intent);
    if let Err(refusal) = gate::admit_cancel(&context) {
        refuse(log, intent, refusal, false);
        return Landing::Refused;
    }
    let actor_id = intent.actor_id().ok_or(Reason::UnknownId);
    match actor_id.and_then(|id| state.actors_mut().take_pending(id)) {
        Ok(_) => Landing::Landed,
        Err(code) => {
            refuse(log, intent, gate::Refusal::new(intent, code), false);
            landing_of(code)
        }
    }
}

/// Leave `rule` pending until tick `until`. A walk first puts the actor on its exit.
#[allow(clippy::too_many_arguments)]
fn pend(
    canon: &Canon,
    state: &mut State,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
    exit: Option<&Exit>,
    until: u64,
    log: &mut RoundLog,
) -> Result<(), Reason> {
    let actor_id = intent.actor_id().ok_or(Reason::UnknownId)?;
    if let Some(exit) = exit {
        let from = (state.actors().stores().place.get(actor_id)).ok_or(Reason::UnknownId)?;
        let kind = EffectKind::Location {
            actor_id: actor_id.clone(),
            from: from.clone(),
            to: travel::on_exit(exit),
            arrived_at: None,
        };
        let effect = Effect::new(intent.actor.as_ref(), &intent.id, kind);
        effect.clone().apply(canon, state, presence, intent)?;
        log.effects.push(effect);
    }
    let pending = PendingAction {
        verb: intent.move_name.clone(),
        source: intent.source,
        target: intent.target.clone(),
        rule: rule.id.clone(),
    };
    state.actors_mut().set_busy(actor_id, until, Some(pending))
}

/// Finish an action at tick `at`: pay the finish cost, count the act when the rule asks, keep
/// the actor busy through the rule's cooldown, then land every effect the rule expands to, a
/// walk's arrival stamped `at`. A refusal is journaled and the round goes on.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish(
    canon: &Canon,
    state: &mut State,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
    rolled: Option<Roll>,
    at: u64,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Landing {
    let context = Context::new(canon, state, presence, intent);
    let roller = dice.roller(log, &rule.id, intent.actor_id());
    let mut expansion = match expand::expand(&context, rule, Cause::Finish, rolled, roller) {
        Ok(expansion) => expansion,
        Err(code) => {
            refuse(log, intent, gate::Refusal::new(intent, code), false);
            return landing_of(code);
        }
    };
    for effect in &mut expansion.effects {
        if let EffectKind::Location { arrived_at, .. } = &mut effect.kind {
            *arrived_at = Some(at);
        }
    }
    let point = state.savepoint();
    let mut roller = dice.roller(log, &rule.id, intent.actor_id());
    let mut finished = match &rule.finish.cost {
        Some(cost) => cost::pay(canon, state, presence, intent, cost, &mut roller),
        None => Ok(()),
    };
    drop(roller);
    if rule.finish.counts_as_act {
        finished = finished.and_then(|()| count_act(state, presence, intent));
    }
    if rule.cooldown > 0 {
        finished = finished.and_then(|()| {
            let actor_id = intent.actor_id().ok_or(Reason::UnknownId)?;
            state
                .actors_mut()
                .set_busy(actor_id, at + rule.cooldown, None)
        });
    }
    if let Err(code) = finished {
        state.rollback(point);
        refuse(log, intent, gate::Refusal::new(intent, code), false);
        return landing_of(code);
    }
    let landing = land(canon, state, presence, intent, rule, expansion, log);
    if !matches!(landing, Landing::Landed) {
        state.rollback(point);
    }
    landing
}

fn count_act(state: &mut State, presence: &Presence, intent: &Intent) -> Result<(), Reason> {
    let actor_id = intent.actor_id().ok_or(Reason::UnknownId)?;
    let round_index = state.round_index;
    (state.actors_mut()).count_act(actor_id, round_index, presence.players)
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
    state: &mut State,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
    expansion: Expansion,
    log: &mut RoundLog,
) -> Landing {
    let cause = expansion.cause;
    let point = state.savepoint();
    let written = expansion.effects.iter().try_for_each(|effect| {
        effect.clone().apply(canon, state, presence, intent)?;
        match cause {
            Cause::Finish => log.extra_steps.push(Step::EffectApplied),
            Cause::Trigger => {}
        }
        Ok(())
    });
    if let Err(code) = written {
        state.rollback(point);
        let refusal = gate::Refusal::new(intent, code);
        match cause {
            Cause::Finish => refuse(log, intent, refusal, false),
            Cause::Trigger => log.refused.push(refusal),
        }
        return landing_of(code);
    }
    log.effects.extend(expansion.effects);
    log.report_roll(expansion.rolled, intent.actor_id());
    log.reports.extend(expansion.resisted);
    log.extra_steps.push(Step::Rule(rule.id.clone()));
    Landing::Landed
}

/// Journal the refusal, and tell the actor its code and detail: a player always, an NPC only
/// when `tell_npcs`.
pub(crate) fn refuse(log: &mut RoundLog, intent: &Intent, refusal: gate::Refusal, tell_npcs: bool) {
    let told = match &intent.actor {
        Some(Viewer::Player(id)) => Some(id),
        Some(Viewer::Npc(id)) if tell_npcs => Some(id),
        _ => None,
    };
    if let Some(actor_id) = told {
        log.reports.push(MoveReport::Refused {
            actor_id: actor_id.clone(),
            refusal: refusal.clone(),
        });
    }
    log.refused.push(refusal);
}
