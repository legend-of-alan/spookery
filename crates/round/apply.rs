use spookery_kernel::apply::apply_effect;
use spookery_kernel::effect::Effect;
use spookery_kernel::expand::expand_outcome;
use spookery_kernel::intent::{ActorKind, Intent};
use spookery_kernel::perceive::perceive_at;
use spookery_kernel::rules::cost;
use spookery_kernel::rules::gate::{self, gate};
use spookery_kernel::rules::{RollReport, RollResult};
use spookery_kernel::world::Draft;
use spookery_kernel::Reason;

use crate::log::Rejected;
use crate::resolve::viewer_token;
use crate::round::{Resolution, Round};

pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    round.trace.push("apply".to_string());
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
            refuse(round, &intent.id, reason);
            continue;
        }
        let outcome = rolled.as_ref().map(|out| out.outcome);
        let expanded = match expand_outcome(draft.canon(), &frozen, &intent, outcome) {
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
        if commit_intent(round, draft, &intent, &expanded, &rolled) && spends_slot {
            slots += 1;
        }
    }
    Ok(())
}

/// Perceive and eligibility see effects already applied in this round.
fn live_gate(draft: &Draft, intent: &Intent) -> Result<Option<&'static str>, Reason> {
    let view = perceive_at(draft.canon(), draft.now(), &viewer_token(intent)?)?;
    if view.known_facts.is_empty() {
        return Ok(Some("empty_perceive"));
    }
    if let Err(code) = gate(draft.canon(), draft.now(), intent) {
        return Ok(Some(code.jsonl_reason()));
    }
    Ok(None)
}

fn commit_intent(
    round: &mut Round,
    draft: &mut Draft,
    intent: &Intent,
    expanded: &[Effect],
    rolled: &Option<RollResult>,
) -> bool {
    let point = draft.savepoint();
    if let Ok(rule) = gate::resolve(draft.canon(), draft.now(), intent) {
        if let Some(cost) = rule.cost.clone() {
            if let Err(code) = cost::pay(draft, &intent.actor_id, &cost) {
                draft.rollback(point);
                refuse(round, &intent.id, code.jsonl_reason());
                return false;
            }
        }
    }
    let mut accepted = Vec::new();
    for effect in expanded {
        match apply_effect(draft, intent, effect.clone()) {
            Ok(()) => {
                round.trace.push("apply_effect".to_string());
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
    round.applied.push(intent.clone());
    true
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
