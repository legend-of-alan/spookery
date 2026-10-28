//! Gate each intent and roll the checks it asks for.

use crate::check::classify;
use crate::rules::expr::{self, Ctx};
use crate::rules::gate::{self, gate};

use crate::draft::Draft;
use crate::intent::{Intent, Source, Target, ViewerId};
use crate::rules::{MoveReport, RollResult};
use crate::Reason;

use crate::round::dice::ResistKey;
use crate::round::state::{Resolution, Round};

pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    round.frozen = draft.now().clone();
    let candidates = round.candidates.clone();
    for intent in candidates {
        let resolution = consider(round, draft, &intent)?;
        round.resolutions.insert(intent.id, resolution);
    }
    Ok(())
}

fn consider(round: &mut Round, draft: &Draft, intent: &Intent) -> Result<Resolution, Reason> {
    let rule = match gate(draft.canon(), draft.now(), intent) {
        Ok(rule) => rule.clone(),
        Err(refusal) => return Ok(refused(refusal, None)),
    };
    if crate::map::span(draft.canon(), draft.now(), intent, &rule) > 0 {
        return Ok(Resolution::Ready { rolled: None });
    }
    if let Some(roll) = rule.roll.clone() {
        round.steps_run.push("resolve_check".to_string());
        return roll_check(round, draft, intent, &roll);
    }
    Ok(Resolution::Ready { rolled: None })
}

fn roll_check(
    round: &mut Round,
    draft: &Draft,
    intent: &Intent,
    roll: &crate::rule::RollSpec,
) -> Result<Resolution, Reason> {
    if let Err(code) = ViewerId::from_kind(intent.actor_kind, &intent.actor_id) {
        return Ok(refused_check(intent, code));
    }
    let opposed = match supplied_target(round, draft, intent) {
        Ok(total) => total,
        Err(code) => return Ok(refused_check(intent, code)),
    };
    let (target, partial_at) = {
        let mut roller = round
            .dice
            .roller(&intent.move_name, &intent.actor_id, ResistKey::Rule);
        let ctx = Ctx { intent };
        let partial_at = if opposed.is_some() {
            None
        } else {
            match roll.partial_at.as_ref() {
                Some(expr) => Some(
                    expr::eval_with_dice(draft.canon(), &round.frozen, &ctx, expr, &mut roller)?
                        .value,
                ),
                None => None,
            }
        };
        let target = match opposed {
            Some(total) => total,
            None => {
                expr::eval_with_dice(
                    draft.canon(),
                    &round.frozen,
                    &ctx,
                    &roll.target,
                    &mut roller,
                )?
                .value
            }
        };
        (target, partial_at)
    };
    if let Some(out) = round.opposed.get(&intent.id).cloned() {
        return Ok(Resolution::Ready {
            rolled: Some(reclassify(&out, target, partial_at)),
        });
    }
    let key = round.dice.key(&intent.move_name, &intent.actor_id);
    match round.dice.check(
        draft.canon(),
        &round.frozen,
        key,
        &roll.check,
        target,
        partial_at,
    ) {
        Ok(out) => {
            round.opposed.insert(intent.id.clone(), out.clone());
            Ok(Resolution::Ready { rolled: Some(out) })
        }
        Err(code) => Ok(refused_check(intent, code)),
    }
}

fn supplied_target(
    round: &mut Round,
    draft: &Draft,
    intent: &Intent,
) -> Result<Option<i64>, Reason> {
    let Target::Actor(named) = &intent.target else {
        return Ok(None);
    };
    let named = named.clone();
    let opposing = round
        .opposing_candidates
        .iter()
        .find(|other| {
            other.id != intent.id
                && other.actor_id == named
                && other.source == Source::Player
                && gate(draft.canon(), draft.now(), other)
                    .ok()
                    .is_some_and(|rule| rule.roll.is_some())
        })
        .cloned();
    let Some(opposing) = opposing else {
        return Ok(None);
    };
    if let Some(out) = round.opposed.get(&opposing.id) {
        return Ok(Some(out.total));
    }
    let Some(check_id) = gate(draft.canon(), draft.now(), &opposing)
        .ok()
        .and_then(|rule| rule.roll.as_ref().map(|roll| roll.check.clone()))
    else {
        return Ok(None);
    };
    let key = round.dice.key(&opposing.move_name, &opposing.actor_id);
    let out = round
        .dice
        .check(draft.canon(), &round.frozen, key, &check_id, 0, None)?;
    let total = out.total;
    round.opposed.insert(opposing.id, out);
    Ok(Some(total))
}

fn reclassify(out: &RollResult, target: i64, partial_at: Option<i64>) -> RollResult {
    RollResult {
        faces: out.faces.clone(),
        stat: out.stat,
        total: out.total,
        target,
        outcome: classify(out.total, target, partial_at),
    }
}

fn refused(refusal: gate::GateRefusal, report: Option<MoveReport>) -> Resolution {
    Resolution::Refused { refusal, report }
}

fn refused_check(intent: &Intent, code: Reason) -> Resolution {
    refused(
        gate::GateRefusal::Code(code),
        Some(MoveReport::Refused {
            actor_id: intent.actor_id.clone(),
            code,
            missing: None,
        }),
    )
}
