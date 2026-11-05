//! Pay each intent cost and write the effects it earned.

use crate::canon::Canon;
use crate::draft::Draft;
use crate::intent::{Intent, Viewer};
use crate::model::State;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::expand::{self, Cause, Expansion};
use crate::rules::gate;
use crate::rules::roll::MoveReport;
use crate::rules::rule::Rule;
use crate::rules::travel;
use crate::store::records::Step;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;
use crate::round::stage::advance::RoundTime;
use crate::round::stage::resolve::{Candidate, Resolution};

/// What became of one rule's effects in `land`.
pub(crate) enum Landing {
    Landed,
    Refused,
    /// Refused `BUDGET`: the round's state keys are spent.
    OverBudget,
}

pub(crate) fn run(
    draft: &mut Draft,
    canon: &Canon,
    state: &State,
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
        let (rule, rolled) = match resolution {
            Resolution::Ready { rule, rolled } => (rule, rolled),
            Resolution::Refused(refusal) => {
                refuse(log, intent, refusal, false);
                continue;
            }
            Resolution::CannotRoll(code) => {
                refuse(log, intent, gate::Refusal::new(intent, code), true);
                continue;
            }
        };
        let actor_id = intent.actor_id();
        let context = Context::new(canon, draft.state(), draft.presence, intent);
        if let Err(refusal) = gate::admit(&context, rule) {
            refuse(log, intent, refusal, false);
            continue;
        }
        // A timed move expands when it finishes; an instant one against the state it rolled on.
        let minutes = travel::span(&context, rule);
        let expansion = if minutes > 0 {
            None
        } else {
            let context = Context::new(canon, state, draft.presence, intent);
            let roller = dice.roller(log, &rule.id, actor_id);
            match expand::expand(&context, rule, Cause::Move, rolled, roller) {
                Ok(expansion) => Some(expansion),
                Err(code) => {
                    refuse(log, intent, gate::Refusal::new(intent, code), false);
                    continue;
                }
            }
        };
        let point = draft.savepoint();
        if let Some(cost) = &rule.cost {
            let mut roller = dice.roller(log, &rule.id, actor_id);
            if let Err(code) = cost::pay(draft, intent, cost, &mut roller) {
                draft.rollback(point);
                refuse(log, intent, gate::Refusal::new(intent, code), false);
                continue;
            }
        }
        let landing = match expansion {
            Some(expansion) => land(draft, intent, rule, expansion, log),
            None => {
                let at = clock.world_time_before;
                let mut roller = dice.roller(log, &rule.id, actor_id);
                match travel::begin_action(draft, intent, rule, at, minutes, &mut roller) {
                    Ok(()) => {
                        log.extra_steps.push(Step::Rule(rule.id.clone()));
                        Landing::Landed
                    }
                    Err(code) => {
                        refuse(log, intent, gate::Refusal::new(intent, code), false);
                        Landing::Refused
                    }
                }
            }
        };
        // The cost goes back with the effects.
        match landing {
            Landing::Landed if spends_slot => slots += 1,
            Landing::Landed => {}
            Landing::Refused => draft.rollback(point),
            Landing::OverBudget => {
                draft.rollback(point);
                keys_spent = true;
            }
        }
    }
}

/// The one landing for a rule's effects: a move, a timed finish or a change rule. Under one
/// savepoint it writes each effect, then counts a moving NPC's act. A failed write rolls back
/// and refuses the intent; otherwise the effects and the rolls are kept and the rule takes its
/// `Rule` step. The expansion's `Cause` decides the steps, the act and who hears a refusal.
pub(crate) fn land(
    draft: &mut Draft,
    intent: &Intent,
    rule: &Rule,
    expansion: Expansion,
    log: &mut RoundLog,
) -> Landing {
    let cause = expansion.cause;
    let point = draft.savepoint();
    let written = expansion.effects.iter().try_for_each(|effect| {
        effect.clone().apply(draft, intent)?;
        match cause {
            Cause::Move | Cause::Finish => log.extra_steps.push(Step::EffectApplied),
            Cause::Trigger => {}
        }
        Ok(())
    });
    let written = written.and_then(|()| match (cause, &intent.actor) {
        (Cause::Move, Some(Viewer::Npc(id))) => draft.count_npc_act(id, draft.presence.players),
        (Cause::Move | Cause::Finish | Cause::Trigger, _) => Ok(()),
    });
    if let Err(code) = written {
        draft.rollback(point);
        let refusal = gate::Refusal::new(intent, code);
        match cause {
            Cause::Move | Cause::Finish => refuse(log, intent, refusal, false),
            Cause::Trigger => log.refused.push(refusal),
        }
        return if code == Reason::Budget {
            Landing::OverBudget
        } else {
            Landing::Refused
        };
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
