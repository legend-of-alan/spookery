use spookery_kernel::api::apply::apply_effect;
use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::effect::Effect;
use spookery_kernel::api::expand::expand_outcome;
use spookery_kernel::api::intent::{ActorKind, Intent};
use spookery_kernel::api::journal::Rejected;
use spookery_kernel::api::map;
use spookery_kernel::api::perceive::perceive_at;
use spookery_kernel::api::rules::cost;
use spookery_kernel::api::rules::gate::{self, gate};
use spookery_kernel::api::rules::{RollReport, RollResult};
use spookery_kernel::api::Reason;

use super::resolve::{roll_subject, viewer_token, SubjectRoll};
use crate::state::{Resolution, Round};

pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    round.steps_run.push("apply".to_string());
    let max_effects = draft.budget().max_effects_per_intent as usize;
    let max_npcs = draft.budget().max_npcs_acting;
    let mut slots = 0u64;
    let frozen = round.frozen.clone();
    let checks = draft.canon().checks.clone();
    let meters = draft.canon().meters.clone();
    let candidates = round.candidates.clone();
    for intent in candidates {
        if round.budget_stop {
            break;
        }
        let spends_slot = intent.actor_kind == ActorKind::Npc;
        if spends_slot && slots >= max_npcs {
            refuse(round, &intent.id, Reason::Budget.jsonl_reason());
            continue;
        }
        let Some(resolution) = round.resolutions.get(&intent.id).cloned() else {
            continue;
        };
        if let Resolution::Refused { reason, notice } = resolution {
            round.rejected.push(Rejected {
                id: intent.id,
                reason,
            });
            if let Some(notice) = notice {
                round.roll_reports.push(notice);
            }
            continue;
        }
        let Resolution::Ready { rolled } = resolution else {
            continue;
        };
        if let Some(reason) = live_gate(draft, &intent)? {
            refuse(round, &intent.id, &reason);
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
        let mut resist_reports = Vec::new();
        let expanded = match expand_outcome(
            draft.canon(),
            &frozen,
            &intent,
            outcome,
            |actor_id, check_id, target| {
                let out = resist_roll(
                    round,
                    &ResistCtx {
                        checks: &checks,
                        meters: &meters,
                        frozen: &frozen,
                        intent: &intent,
                    },
                    actor_id,
                    check_id,
                    target,
                )?;
                let band = out.outcome;
                resist_reports.push(RollReport::Rolled {
                    actor_id: actor_id.to_string(),
                    out,
                });
                Ok(band)
            },
        ) {
            Ok(effects) => effects,
            Err(code) => {
                refuse(round, &intent.id, code.jsonl_reason());
                continue;
            }
        };
        if expanded.len() > max_effects {
            refuse(round, &intent.id, Reason::Budget.jsonl_reason());
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
    rule: &spookery_kernel::api::rule::Rule,
    minutes: u64,
) -> bool {
    let point = draft.savepoint();
    if let Some(cost) = &rule.cost {
        if let Err(code) = cost::pay(draft, &intent.actor_id, cost) {
            draft.rollback(point);
            refuse(round, &intent.id, code.jsonl_reason());
            return false;
        }
    }
    if let Err(code) = map::begin(draft, intent, rule, round.world_time_before, minutes) {
        draft.rollback(point);
        refuse(round, &intent.id, code.jsonl_reason());
        return false;
    }
    round.steps_run.push(rule.id.clone());
    round.applied.push(intent.clone());
    true
}

/// Perceive and eligibility see effects already applied in this round.
fn live_gate(draft: &Draft, intent: &Intent) -> Result<Option<String>, Reason> {
    let view = perceive_at(draft.canon(), draft.now(), &viewer_token(intent)?)?;
    if view.known_facts.is_empty() {
        return Ok(Some("empty_perceive".to_string()));
    }
    if let Err(miss) = gate(draft.canon(), draft.now(), intent) {
        return Ok(Some(miss.named()));
    }
    Ok(None)
}

fn commit_intent(
    round: &mut Round,
    draft: &mut Draft,
    intent: &Intent,
    expanded: &[Effect],
    rolled: &Option<RollResult>,
    resist_reports: Vec<RollReport>,
) -> bool {
    let point = draft.savepoint();
    let (rule_id, cost) = match gate::resolve(draft.canon(), draft.now(), intent) {
        Ok(rule) => (Some(rule.id.clone()), rule.cost.clone()),
        Err(_) => (None, None),
    };
    if let Some(cost) = cost {
        if let Err(code) = cost::pay(draft, &intent.actor_id, &cost) {
            draft.rollback(point);
            refuse(round, &intent.id, code.jsonl_reason());
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
                refuse(round, &intent.id, code.jsonl_reason());
                if code == Reason::Budget {
                    round.budget_stop = true;
                }
                return false;
            }
        }
    }
    if intent.actor_kind == ActorKind::Npc {
        if let Err(code) = draft.stage_npc_counters(&intent.actor_id, round.humans) {
            draft.rollback(point);
            refuse(round, &intent.id, code.jsonl_reason());
            if code == Reason::Budget {
                round.budget_stop = true;
            }
            return false;
        }
    }
    round.effects.extend(accepted);
    keep_roll(round, rolled, &intent.actor_id);
    round.roll_reports.extend(resist_reports);
    if let Some(rule_id) = rule_id {
        round.steps_run.push(rule_id);
    }
    round.applied.push(intent.clone());
    true
}

struct ResistCtx<'a> {
    checks: &'a std::collections::BTreeMap<String, spookery_kernel::api::check::CheckDef>,
    meters: &'a std::collections::BTreeMap<String, spookery_kernel::api::meter::MeterDef>,
    frozen: &'a spookery_kernel::api::model::Now,
    intent: &'a Intent,
}

fn resist_roll(
    round: &mut Round,
    ctx: &ResistCtx<'_>,
    actor_id: &str,
    check_id: &str,
    target: i64,
) -> Result<RollResult, Reason> {
    let kind = if ctx
        .frozen
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .is_pc()
    {
        ActorKind::Player
    } else {
        ActorKind::Npc
    };
    roll_subject(
        round,
        ctx.checks,
        ctx.meters,
        &SubjectRoll {
            rule_id: &ctx.intent.move_name,
            actor_kind: kind,
            actor_id,
            check_id,
            target,
            partial_at: None,
            live: None,
        },
    )
}

fn refuse(round: &mut Round, id: &str, reason: &str) {
    round.rejected.push(Rejected {
        id: id.to_string(),
        reason: reason.to_string(),
    });
}

fn keep_roll(round: &mut Round, rolled: &Option<RollResult>, actor_id: &str) {
    if let Some(out) = rolled.clone() {
        round.roll_reports.push(RollReport::Rolled {
            actor_id: actor_id.to_string(),
            out,
        });
    }
}
