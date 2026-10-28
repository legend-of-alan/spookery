//! Pay each intent cost and write the effects it earned.

use crate::apply::apply_effect;
use crate::draft::Draft;
use crate::effect::Effect;
use crate::expand::expand_outcome;
use crate::intent::{ActorKind, Intent};
use crate::journal::Rejected;
use crate::map;
use crate::model::Now;
use crate::rules::cost;
use crate::rules::gate::{self, gate};
use crate::rules::{MoveReport, RollResult};
use crate::Reason;

use crate::round::dice::{ResistKey, RoundDice};
use crate::round::log::RoundLog;
use crate::round::stage::advance::Clock;
use crate::round::stage::resolve::{Resolution, Resolved};

/// What became of one intent in apply.
enum Landing {
    Landed,
    Refused,
    BudgetSpent,
}

pub(crate) fn run(
    draft: &mut Draft,
    snapshot: &Now,
    resolved: Vec<Resolved>,
    clock: &Clock,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<(), Reason> {
    let max_effects = draft.budget().max_effects_per_intent as usize;
    let max_npcs = draft.budget().max_npcs_acting;
    let mut slots = 0u64;
    for Resolved { intent, resolution } in resolved {
        let spends_slot = intent.actor_kind == ActorKind::Npc;
        if spends_slot && slots >= max_npcs {
            refuse(log, &intent, Reason::Budget);
            continue;
        }
        let rolled = match resolution {
            Resolution::Ready { rolled } => rolled,
            Resolution::Refused { refusal, report } => {
                log.rejected.push(Rejected {
                    id: intent.id.clone(),
                    reason: refusal.named(),
                });
                if let Some(report) = report {
                    log.reports.push(report);
                } else {
                    report_player(log, &intent, &refusal);
                }
                continue;
            }
        };
        if let Err(refusal) = gate(draft.canon(), draft.now(), &intent) {
            reject(log, &intent, &refusal);
            continue;
        }
        let timed = gate::resolve(draft.canon(), draft.now(), &intent)
            .ok()
            .map(|rule| {
                let minutes = map::span(draft.canon(), draft.now(), &intent, rule);
                (rule.clone(), minutes)
            });
        if let Some((rule, minutes)) = timed {
            if minutes > 0 {
                let at = clock.world_time_before;
                if !begin_timed(draft, &intent, &rule, at, minutes, dice, log) {
                    continue;
                }
                if spends_slot {
                    slots += 1;
                }
                continue;
            }
        }
        let outcome = rolled.as_ref().map(|out| out.outcome);
        let mut roller = dice.roller(log, &intent.move_name, &intent.actor_id, ResistKey::Rule);
        let expanded = expand_outcome(draft.canon(), snapshot, &intent, outcome, &mut roller);
        let resisted = roller.finish();
        let expanded = match expanded {
            Ok(effects) => effects,
            Err(code) => {
                refuse(log, &intent, code);
                continue;
            }
        };
        if expanded.len() > max_effects {
            refuse(log, &intent, Reason::Budget);
            continue;
        }
        match commit_intent(draft, &intent, &expanded, &rolled, resisted, dice, log) {
            Landing::Landed if spends_slot => slots += 1,
            Landing::Landed | Landing::Refused => {}
            Landing::BudgetSpent => break,
        }
    }
    super::complete::finish(draft, clock.world_time_after, dice, log)
}

fn begin_timed(
    draft: &mut Draft,
    intent: &Intent,
    rule: &crate::rule::Rule,
    at: u64,
    minutes: u64,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> bool {
    let point = draft.savepoint();
    let mut roller = dice.roller(log, &intent.move_name, &intent.actor_id, ResistKey::Rule);
    let begun = match &rule.cost {
        Some(cost) => cost::pay(draft, intent, cost, &mut roller),
        None => Ok(()),
    }
    .and_then(|()| map::begin(draft, intent, rule, at, minutes, &mut roller));
    drop(roller);
    if let Err(code) = begun {
        draft.rollback(point);
        refuse(log, intent, code);
        return false;
    }
    log.steps.push(rule.id.clone());
    log.applied.push(intent.clone());
    true
}

fn commit_intent(
    draft: &mut Draft,
    intent: &Intent,
    expanded: &[Effect],
    rolled: &Option<RollResult>,
    resisted: Vec<MoveReport>,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Landing {
    let point = draft.savepoint();
    let (rule_id, cost) = match gate::resolve(draft.canon(), draft.now(), intent) {
        Ok(rule) => (Some(rule.id.clone()), rule.cost.clone()),
        Err(_) => (None, None),
    };
    if let Some(cost) = cost {
        let mut roller = dice.roller(log, &intent.move_name, &intent.actor_id, ResistKey::Rule);
        let paid = cost::pay(draft, intent, &cost, &mut roller);
        drop(roller);
        if let Err(code) = paid {
            draft.rollback(point);
            refuse(log, intent, code);
            return Landing::Refused;
        }
    }
    let staged = expanded.iter().try_for_each(|effect| {
        apply_effect(draft, intent, effect.clone())?;
        log.steps.push("apply_effect".to_string());
        Ok(())
    });
    let staged = staged.and_then(|()| match intent.actor_kind {
        ActorKind::Npc => draft.stage_npc_counters(&intent.actor_id, draft.now().presence.humans),
        _ => Ok(()),
    });
    if let Err(code) = staged {
        draft.rollback(point);
        refuse(log, intent, code);
        return if code == Reason::Budget {
            Landing::BudgetSpent
        } else {
            Landing::Refused
        };
    }
    log.effects.extend_from_slice(expanded);
    keep_roll(log, rolled, &intent.actor_id);
    log.reports.extend(resisted);
    if let Some(rule_id) = rule_id {
        log.steps.push(rule_id);
    }
    log.applied.push(intent.clone());
    Landing::Landed
}

fn refuse(log: &mut RoundLog, intent: &Intent, code: Reason) {
    reject(log, intent, &gate::GateRefusal::Code(code));
}

/// Journal the refusal's word and tell a player its code and name.
fn reject(log: &mut RoundLog, intent: &Intent, refusal: &gate::GateRefusal) {
    log.rejected.push(Rejected {
        id: intent.id.clone(),
        reason: refusal.named(),
    });
    report_player(log, intent, refusal);
}

fn report_player(log: &mut RoundLog, intent: &Intent, refusal: &gate::GateRefusal) {
    if intent.actor_kind != ActorKind::Player {
        return;
    }
    log.reports.push(MoveReport::Refused {
        actor_id: intent.actor_id.clone(),
        code: refusal.reason(),
        missing: refusal.missing().map(str::to_string),
    });
}

fn keep_roll(log: &mut RoundLog, rolled: &Option<RollResult>, actor_id: &str) {
    if let Some(out) = rolled.clone() {
        log.reports.push(MoveReport::Rolled {
            actor_id: actor_id.to_string(),
            out,
        });
    }
}
