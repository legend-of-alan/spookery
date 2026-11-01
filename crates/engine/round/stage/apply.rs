//! Pay each intent cost and write the effects it earned.

use crate::canon::Canon;
use crate::draft::Draft;
use crate::intent::{Intent, Viewer};
use crate::journal::Step;
use crate::model::State;
use crate::rule::{Rule, When};
use crate::rules::case::Case;
use crate::rules::cost;
use crate::rules::expand::{self, Expansion};
use crate::rules::gate;
use crate::rules::MoveReport;
use crate::travel;
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
    snapshot: &State,
    candidates: Vec<Candidate>,
    clock: &RoundTime,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) {
    let max_npcs = canon.budget.max_npcs_acting;
    let mut slots = 0u64;
    for Candidate { intent, resolution } in candidates {
        let spends_slot = matches!(intent.actor, Viewer::Npc(_));
        if spends_slot && slots >= max_npcs {
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
        let actor_id = intent.actor.actor_id();
        let case = Case::new(canon, draft.state(), draft.presence, intent);
        if let Err(refusal) = gate::admit(&case, rule) {
            refuse(log, intent, refusal, false);
            continue;
        }
        // A timed move expands when it finishes; an instant one against the snapshot it rolled on.
        let minutes = travel::span(&case, rule);
        let expansion = if minutes > 0 {
            None
        } else {
            let case = Case::new(canon, snapshot, draft.presence, intent);
            let roller = dice.roller(log, &rule.id, actor_id);
            match expand::expand(&case, rule, rolled, roller) {
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
            Some(expansion) => land(draft, intent, rule, expansion, true, log),
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
                break;
            }
        }
    }
}

/// The one landing for a rule's effects: a move, a timed finish or a change rule. Under one
/// savepoint it writes each effect with an `EffectApplied` step, then counts an NPC's act when
/// `counts_act` (a move; not a finish or a change rule). A failed write rolls back and refuses
/// the intent; otherwise the effects and the rolls are kept and the rule takes its `Rule` step.
/// A change rule is nobody's move: its effects take no step and its refusal is told to nobody.
pub(crate) fn land(
    draft: &mut Draft,
    intent: &Intent,
    rule: &Rule,
    expansion: Expansion,
    counts_act: bool,
    log: &mut RoundLog,
) -> Landing {
    let change_rule = matches!(rule.when, When::Change(_));
    let point = draft.savepoint();
    let written = expansion.effects.iter().try_for_each(|effect| {
        effect.clone().apply(draft, intent)?;
        if !change_rule {
            log.extra_steps.push(Step::EffectApplied);
        }
        Ok(())
    });
    let written = written.and_then(|()| match &intent.actor {
        Viewer::Npc(id) if counts_act => draft.count_npc_act(id, draft.presence.players),
        Viewer::Npc(_) | Viewer::Player(_) => Ok(()),
    });
    if let Err(code) = written {
        draft.rollback(point);
        let refusal = gate::Refusal::new(intent, code);
        if change_rule {
            log.refused.push(refusal);
        } else {
            refuse(log, intent, refusal, false);
        }
        return if code == Reason::Budget {
            Landing::OverBudget
        } else {
            Landing::Refused
        };
    }
    log.effects.extend(expansion.effects);
    log.report_roll(expansion.rolled, intent.actor.actor_id());
    log.reports.extend(expansion.resisted);
    log.extra_steps.push(Step::Rule(rule.id.clone()));
    Landing::Landed
}

/// Journal the refusal, and tell the actor its code and detail: a player always, an NPC only
/// when `tell_npcs`.
pub(crate) fn refuse(log: &mut RoundLog, intent: &Intent, refusal: gate::Refusal, tell_npcs: bool) {
    if tell_npcs || matches!(intent.actor, Viewer::Player(_)) {
        log.reports.push(MoveReport::Refused {
            actor_id: intent.actor.actor_id().to_string(),
            refusal: refusal.clone(),
        });
    }
    log.refused.push(refusal);
}
