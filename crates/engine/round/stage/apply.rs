//! Pay each intent cost and write the effects it earned.

use crate::draft::Draft;
use crate::intent::{Intent, Viewer};
use crate::journal::{Rejected, Step};
use crate::model::State;
use crate::rule::Rule;
use crate::rules::cost;
use crate::rules::gate::{admit, GateRefusal};
use crate::rules::{MoveReport, Roll};
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
            reject(log, &intent, Reason::Budget);
            continue;
        }
        let (rule_id, rolled) = match resolution {
            Resolution::Ready { rule_id, rolled } => (rule_id, rolled),
            Resolution::Refused(refusal) => {
                reject(log, &intent, refusal);
                continue;
            }
            Resolution::CannotRoll(code) => {
                let refusal = GateRefusal::from(code);
                log.rejected.push(Rejected::new(&intent, &refusal));
                log.reports.push(refused_report(&intent, &refusal));
                continue;
            }
        };
        let canon = draft.shared_canon();
        let rule = canon.rules.get(&rule_id).ok_or(Reason::UnknownMove)?;
        if let Err(refusal) = admit(&canon, draft.state(), draft.presence, &intent, rule) {
            reject(log, &intent, refusal);
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
                reject(log, &intent, code);
                continue;
            }
        };
        if effects.len() > max_effects {
            reject(log, &intent, Reason::Budget);
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
        reject(log, intent, code);
        return false;
    }
    log.steps.push(Step::Rule(rule.id.clone()));
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
            reject(log, intent, code);
            return Landing::Refused;
        }
    }
    let staged = expansion.effects.iter().try_for_each(|effect| {
        effect.clone().apply(draft, intent)?;
        log.steps.push(Step::EffectApplied);
        Ok(())
    });
    let staged = staged.and_then(|()| match &intent.actor {
        Viewer::Npc(id) => draft.count_npc_act(id, draft.presence.humans),
        Viewer::Player(_) => Ok(()),
    });
    if let Err(code) = staged {
        draft.rollback(point);
        reject(log, intent, code);
        return if code == Reason::Budget {
            Landing::BudgetSpent
        } else {
            Landing::Refused
        };
    }
    log.effects.extend_from_slice(&expansion.effects);
    report_roll(log, &expansion.rolled, intent.actor.actor_id());
    log.reports.extend_from_slice(&expansion.resisted);
    log.steps.push(Step::Rule(rule.id.clone()));
    Landing::Applied
}

/// Journal the refusal and tell a player its code and what was missing.
fn reject(log: &mut RoundLog, intent: &Intent, refusal: impl Into<GateRefusal>) {
    let refusal = refusal.into();
    log.rejected.push(Rejected::new(intent, &refusal));
    if matches!(intent.actor, Viewer::Player(_)) {
        log.reports.push(refused_report(intent, &refusal));
    }
}

fn refused_report(intent: &Intent, refusal: &GateRefusal) -> MoveReport {
    MoveReport::Refused {
        actor_id: intent.actor.actor_id().to_string(),
        code: refusal.reason(),
        missing: refusal.missing().map(str::to_string),
    }
}

fn report_roll(log: &mut RoundLog, rolled: &Option<Roll>, actor_id: &str) {
    if let Some(out) = rolled.clone() {
        log.reports.push(MoveReport::Rolled {
            actor_id: actor_id.to_string(),
            roll: out,
        });
    }
}
