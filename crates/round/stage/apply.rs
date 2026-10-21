//! Pay each intent cost and write the effects it earned.

use spookery_kernel::api::apply::apply_effect;
use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::effect::Effect;
use spookery_kernel::api::expand::expand_outcome;
use spookery_kernel::api::intent::{ActorKind, Intent};
use spookery_kernel::api::journal::Rejected;
use spookery_kernel::api::map;
use spookery_kernel::api::rules::cost;
use spookery_kernel::api::rules::gate::{self, gate};
use spookery_kernel::api::rules::{MoveReport, RollResult};
use spookery_kernel::api::Reason;

use super::resolve::{roll_with_key, SubjectRoll};
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
            refuse(round, &intent, Reason::Budget);
            continue;
        }
        let Some(resolution) = round.resolutions.get(&intent.id).cloned() else {
            continue;
        };
        if let Resolution::Refused { miss, report } = resolution {
            round.rejected.push(Rejected {
                id: intent.id.clone(),
                reason: miss.named(),
            });
            if let Some(report) = report {
                round.move_reports.push(report);
            } else {
                report_player(round, &intent, &miss);
            }
            continue;
        }
        let Resolution::Ready { rolled } = resolution else {
            continue;
        };
        if let Err(miss) = gate(draft.canon(), draft.now(), &intent) {
            reject(round, &intent, &miss);
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
        let expanded = match super::resolve::with_dice(round, |held| {
            let mut roller = held.roller(&intent.move_name, &intent.actor_id);
            expand_outcome(
                draft.canon(),
                &frozen,
                &intent,
                outcome,
                &mut roller,
                |actor_id, check_id, target| {
                    let key = held.key(&intent.move_name, actor_id);
                    let out = resist_roll(
                        &frozen,
                        &ResistCtx {
                            checks: &checks,
                            meters: &meters,
                            intent: &intent,
                        },
                        &key,
                        actor_id,
                        check_id,
                        target,
                    )?;
                    let band = out.outcome;
                    resist_reports.push(MoveReport::Rolled {
                        actor_id: actor_id.to_string(),
                        out,
                    });
                    Ok(band)
                },
            )
        }) {
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
    rule: &spookery_kernel::api::rule::Rule,
    minutes: u64,
) -> bool {
    let point = draft.savepoint();
    let at = round.world_time_before;
    if let Err(code) = super::resolve::with_dice(round, |held| {
        let mut roller = held.roller(&intent.move_name, &intent.actor_id);
        if let Some(cost) = &rule.cost {
            cost::pay(draft, intent, cost, &mut roller)?;
        }
        map::begin(draft, intent, rule, at, minutes, &mut roller)
    }) {
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
        if let Err(code) = super::resolve::with_dice(round, |held| {
            let mut roller = held.roller(&intent.move_name, &intent.actor_id);
            cost::pay(draft, intent, &cost, &mut roller)
        }) {
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
        if let Err(code) = draft.stage_npc_counters(&intent.actor_id, round.humans) {
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

struct ResistCtx<'a> {
    checks: &'a std::collections::BTreeMap<String, spookery_kernel::api::check::CheckDef>,
    meters: &'a std::collections::BTreeMap<String, spookery_kernel::api::meter::MeterDef>,
    intent: &'a Intent,
}

fn resist_roll(
    now: &spookery_kernel::api::model::Now,
    ctx: &ResistCtx<'_>,
    key: &spookery_kernel::api::rules::RollKey,
    actor_id: &str,
    check_id: &str,
    target: i64,
) -> Result<RollResult, Reason> {
    let kind = if now.actors.get(actor_id).ok_or(Reason::UnknownKey)?.is_pc() {
        ActorKind::Player
    } else {
        ActorKind::Npc
    };
    roll_with_key(
        now,
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
        key,
    )
}

fn refuse(round: &mut Round, intent: &Intent, code: Reason) {
    reject(round, intent, &gate::Miss::Code(code));
}

/// Journal the miss's word and tell a player its code and name.
fn reject(round: &mut Round, intent: &Intent, miss: &gate::Miss) {
    round.rejected.push(Rejected {
        id: intent.id.clone(),
        reason: miss.named(),
    });
    report_player(round, intent, miss);
}

fn report_player(round: &mut Round, intent: &Intent, miss: &gate::Miss) {
    if intent.actor_kind != ActorKind::Player {
        return;
    }
    round.move_reports.push(MoveReport::Refused {
        actor_id: intent.actor_id.clone(),
        code: miss.reason(),
        missing: miss.missing().map(str::to_string),
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
