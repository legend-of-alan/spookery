//! Pay each intent cost and write the effects it earned.

use crate::apply::apply_effect;
use crate::draft::Draft;
use crate::effect::Effect;
use crate::expand::expand_outcome;
use crate::intent::{ActorKind, Intent};
use crate::journal::Rejected;
use crate::map;
use crate::rules::cost;
use crate::rules::gate::{self, gate};
use crate::rules::{MoveReport, RollResult};
use crate::Reason;

use crate::round::dice::ResistKey;
use crate::round::state::{Resolution, Round};

pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    let max_effects = draft.budget().max_effects_per_intent as usize;
    let max_npcs = draft.budget().max_npcs_acting;
    let mut slots = 0u64;
    let frozen = round.frozen.clone();
    let candidates = round.candidates.clone();
    for intent in candidates {
        if round.budget_stop {
            break;
        }
        let spends_slot = intent.actor_kind == ActorKind::Npc;
        if spends_slot && slots >= max_npcs {
            refuse(round, &intent, Reason::Budget);
            continue;
        }
        let Some(resolution) = round.resolutions.get(&intent.id).cloned() else {
            continue;
        };
        if let Resolution::Refused { refusal, report } = resolution {
            round.rejected.push(Rejected {
                id: intent.id.clone(),
                reason: refusal.named(),
            });
            if let Some(report) = report {
                round.move_reports.push(report);
            } else {
                report_player(round, &intent, &refusal);
            }
            continue;
        }
        let Resolution::Ready { rolled } = resolution else {
            continue;
        };
        if let Err(refusal) = gate(draft.canon(), draft.now(), &intent) {
            reject(round, &intent, &refusal);
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
                if !begin_timed(round, draft, &intent, &rule, minutes) {
                    continue;
                }
                if spends_slot {
                    slots += 1;
                }
                continue;
            }
        }
        let outcome = rolled.as_ref().map(|out| out.outcome);
        let mut roller = round
            .dice
            .roller(&intent.move_name, &intent.actor_id, ResistKey::Rule);
        let expanded = expand_outcome(draft.canon(), &frozen, &intent, outcome, &mut roller);
        let resist_reports = roller.finish();
        let expanded = match expanded {
            Ok(effects) => effects,
            Err(code) => {
                refuse(round, &intent, code);
                continue;
            }
        };
        if expanded.len() > max_effects {
            refuse(round, &intent, Reason::Budget);
            continue;
        }
        if commit_intent(round, draft, &intent, &expanded, &rolled, resist_reports) && spends_slot {
            slots += 1;
        }
    }
    super::complete::finish(round, draft)?;
    Ok(())
}

fn begin_timed(
    round: &mut Round,
    draft: &mut Draft,
    intent: &Intent,
    rule: &crate::rule::Rule,
    minutes: u64,
) -> bool {
    let point = draft.savepoint();
    let at = round.world_time_before;
    let mut roller = round
        .dice
        .roller(&intent.move_name, &intent.actor_id, ResistKey::Rule);
    let begun = match &rule.cost {
        Some(cost) => cost::pay(draft, intent, cost, &mut roller),
        None => Ok(()),
    }
    .and_then(|()| map::begin(draft, intent, rule, at, minutes, &mut roller));
    drop(roller);
    if let Err(code) = begun {
        draft.rollback(point);
        refuse(round, intent, code);
        return false;
    }
    round.steps_run.push(rule.id.clone());
    round.applied.push(intent.clone());
    true
}

fn commit_intent(
    round: &mut Round,
    draft: &mut Draft,
    intent: &Intent,
    expanded: &[Effect],
    rolled: &Option<RollResult>,
    resist_reports: Vec<MoveReport>,
) -> bool {
    let point = draft.savepoint();
    let (rule_id, cost) = match gate::resolve(draft.canon(), draft.now(), intent) {
        Ok(rule) => (Some(rule.id.clone()), rule.cost.clone()),
        Err(_) => (None, None),
    };
    if let Some(cost) = cost {
        let mut roller = round
            .dice
            .roller(&intent.move_name, &intent.actor_id, ResistKey::Rule);
        let paid = cost::pay(draft, intent, &cost, &mut roller);
        drop(roller);
        if let Err(code) = paid {
            draft.rollback(point);
            refuse(round, intent, code);
            return false;
        }
    }
    let mut accepted = Vec::new();
    for effect in expanded {
        match apply_effect(draft, intent, effect.clone()) {
            Ok(()) => {
                round.steps_run.push("apply_effect".to_string());
                accepted.push(effect.clone());
            }
            Err(code) => {
                draft.rollback(point);
                refuse(round, intent, code);
                if code == Reason::Budget {
                    round.budget_stop = true;
                }
                return false;
            }
        }
    }
    if intent.actor_kind == ActorKind::Npc {
        if let Err(code) = draft.stage_npc_counters(&intent.actor_id, draft.now().presence.humans) {
            draft.rollback(point);
            refuse(round, intent, code);
            if code == Reason::Budget {
                round.budget_stop = true;
            }
            return false;
        }
    }
    round.effects.extend(accepted);
    keep_roll(round, rolled, &intent.actor_id);
    round.move_reports.extend(resist_reports);
    if let Some(rule_id) = rule_id {
        round.steps_run.push(rule_id);
    }
    round.applied.push(intent.clone());
    true
}

fn refuse(round: &mut Round, intent: &Intent, code: Reason) {
    reject(round, intent, &gate::GateRefusal::Code(code));
}

/// Journal the refusal's word and tell a player its code and name.
fn reject(round: &mut Round, intent: &Intent, refusal: &gate::GateRefusal) {
    round.rejected.push(Rejected {
        id: intent.id.clone(),
        reason: refusal.named(),
    });
    report_player(round, intent, refusal);
}

fn report_player(round: &mut Round, intent: &Intent, refusal: &gate::GateRefusal) {
    if intent.actor_kind != ActorKind::Player {
        return;
    }
    round.move_reports.push(MoveReport::Refused {
        actor_id: intent.actor_id.clone(),
        code: refusal.reason(),
        missing: refusal.missing().map(str::to_string),
    });
}

fn keep_roll(round: &mut Round, rolled: &Option<RollResult>, actor_id: &str) {
    if let Some(out) = rolled.clone() {
        round.move_reports.push(MoveReport::Rolled {
            actor_id: actor_id.to_string(),
            out,
        });
    }
}
