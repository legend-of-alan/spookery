//! Pay each intent cost and write the effects it earned.

use crate::draft::Draft;
use crate::intent::{Intent, Viewer};
use crate::journal::Step;
use crate::model::State;
use crate::rule::Rule;
use crate::rules::cost;
use crate::rules::gate;
use crate::rules::MoveReport;
use crate::travel;
use crate::Reason;

use crate::round::dice::{Expansion, RoundDice};
use crate::round::log::RoundLog;
use crate::round::stage::advance::RoundTime;
use crate::round::stage::resolve::{Resolution, Resolved};

/// What became of one intent in apply.
enum Landing {
    Applied,
    Refused,
    BudgetSpent,
}

pub(crate) fn run(
    draft: &mut Draft,
    snapshot: &State,
    resolved: Vec<Resolved>,
    clock: &RoundTime,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<(), Reason> {
    let max_effects = draft.budget().max_effects_per_intent as usize;
    let max_npcs = draft.budget().max_npcs_acting;
    let mut slots = 0u64;
    for Resolved { intent, resolution } in resolved {
        let spends_slot = matches!(intent.actor, Viewer::Npc(_));
        if spends_slot && slots >= max_npcs {
            refuse(
                log,
                &intent,
                gate::Refusal::new(&intent, Reason::Budget),
                false,
            );
            continue;
        }
        let (rule_id, rolled) = match resolution {
            Resolution::Ready { rule_id, rolled } => (rule_id, rolled),
            Resolution::Refused(refusal) => {
                refuse(log, &intent, refusal, false);
                continue;
            }
            Resolution::CannotRoll(code) => {
                refuse(log, &intent, gate::Refusal::new(&intent, code), true);
                continue;
            }
        };
        let canon = draft.shared_canon();
        let rule = canon.rules.get(&rule_id).ok_or(Reason::UnknownMove)?;
        if let Err(refusal) = gate::admit(&canon, draft.state(), draft.presence, &intent, rule) {
            refuse(log, &intent, refusal, false);
            continue;
        }
        let minutes = travel::span(&canon, draft.state(), &intent, rule);
        if minutes > 0 {
            let at = clock.world_time_before;
            if start_timed_action(draft, &intent, rule, at, minutes, dice, log) && spends_slot {
                slots += 1;
            }
            continue;
        }
        let outcome = rolled.as_ref().map(|out| out.outcome);
        let mut roller = dice.roller(log, &rule_id, intent.actor.actor_id());
        let presence = draft.presence;
        let expanded = rule.expand(&canon, snapshot, presence, &intent, outcome, &mut roller);
        let resisted = roller.into_reports();
        let effects = match expanded {
            Ok(effects) => effects,
            Err(code) => {
                refuse(log, &intent, gate::Refusal::new(&intent, code), false);
                continue;
            }
        };
        if effects.len() > max_effects {
            refuse(
                log,
                &intent,
                gate::Refusal::new(&intent, Reason::Budget),
                false,
            );
            continue;
        }
        let expansion = Expansion {
            effects,
            rolled,
            resisted,
        };
        match apply_instant_intent(draft, &intent, rule, &expansion, dice, log) {
            Landing::Applied if spends_slot => slots += 1,
            Landing::Applied | Landing::Refused => {}
            Landing::BudgetSpent => break,
        }
    }
    Ok(())
}

fn start_timed_action(
    draft: &mut Draft,
    intent: &Intent,
    rule: &Rule,
    at: u64,
    minutes: u64,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> bool {
    let point = draft.savepoint();
    let mut roller = dice.roller(log, &rule.id, intent.actor.actor_id());
    let begun = match &rule.cost {
        Some(cost) => cost::pay(draft, intent, cost, &mut roller),
        None => Ok(()),
    }
    .and_then(|()| travel::begin_action(draft, intent, rule, at, minutes, &mut roller));
    drop(roller);
    if let Err(code) = begun {
        draft.rollback(point);
        refuse(log, intent, gate::Refusal::new(intent, code), false);
        return false;
    }
    log.extra_steps.push(Step::Rule(rule.id.clone()));
    true
}

fn apply_instant_intent(
    draft: &mut Draft,
    intent: &Intent,
    rule: &Rule,
    expansion: &Expansion,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Landing {
    let point = draft.savepoint();
    if let Some(cost) = &rule.cost {
        let mut roller = dice.roller(log, &rule.id, intent.actor.actor_id());
        let paid = cost::pay(draft, intent, cost, &mut roller);
        drop(roller);
        if let Err(code) = paid {
            draft.rollback(point);
            refuse(log, intent, gate::Refusal::new(intent, code), false);
            return Landing::Refused;
        }
    }
    let staged = expansion.effects.iter().try_for_each(|effect| {
        effect.clone().apply(draft, intent)?;
        log.extra_steps.push(Step::EffectApplied);
        Ok(())
    });
    let staged = staged.and_then(|()| match &intent.actor {
        Viewer::Npc(id) => draft.count_npc_act(id, draft.presence.players),
        Viewer::Player(_) => Ok(()),
    });
    if let Err(code) = staged {
        draft.rollback(point);
        refuse(log, intent, gate::Refusal::new(intent, code), false);
        return if code == Reason::Budget {
            Landing::BudgetSpent
        } else {
            Landing::Refused
        };
    }
    log.effects.extend_from_slice(&expansion.effects);
    log.report_roll(expansion.rolled.clone(), intent.actor.actor_id());
    log.reports.extend_from_slice(&expansion.resisted);
    log.extra_steps.push(Step::Rule(rule.id.clone()));
    Landing::Applied
}

/// Journal the refusal, and tell the actor its code and detail: a player always, an NPC only
/// when `tell_npcs`.
fn refuse(log: &mut RoundLog, intent: &Intent, refusal: gate::Refusal, tell_npcs: bool) {
    if tell_npcs || matches!(intent.actor, Viewer::Player(_)) {
        log.reports.push(MoveReport::Refused {
            actor_id: intent.actor.actor_id().to_string(),
            refusal: refusal.clone(),
        });
    }
    log.refused.push(refusal);
}
