//! Gate each intent and roll the checks it asks for.

use std::collections::BTreeMap;

use crate::check::classify;
use crate::rules::expr::{self, Ctx};
use crate::rules::gate::{self, gate};

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target, ViewerId};
use crate::model::Now;
use crate::rules::{MoveReport, RollResult};
use crate::Reason;

use crate::round::dice::{ResistKey, RoundDice};
use crate::round::log::RoundLog;

/// One candidate and how resolve left it for apply.
pub(crate) struct Resolved {
    pub intent: Intent,
    pub resolution: Resolution,
}

/// How resolve left an intent for apply.
pub(crate) enum Resolution {
    Ready {
        rolled: Option<RollResult>,
    },
    Refused {
        refusal: gate::GateRefusal,
        report: Option<MoveReport>,
    },
}

/// What every roll in the round shares.
struct Rolls<'a> {
    canon: &'a Canon,
    snapshot: &'a Now,
    pool: &'a [Intent],
    opposed: BTreeMap<String, RollResult>,
    dice: &'a mut RoundDice,
    log: &'a mut RoundLog,
}

pub(crate) fn run(
    canon: &Canon,
    snapshot: &Now,
    candidates: Vec<Intent>,
    pool: &[Intent],
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<Vec<Resolved>, Reason> {
    let mut rolls = Rolls {
        canon,
        snapshot,
        pool,
        opposed: BTreeMap::new(),
        dice,
        log,
    };
    let mut resolved = Vec::with_capacity(candidates.len());
    for intent in candidates {
        let resolution = consider(&mut rolls, &intent)?;
        resolved.push(Resolved { intent, resolution });
    }
    Ok(resolved)
}

fn consider(rolls: &mut Rolls<'_>, intent: &Intent) -> Result<Resolution, Reason> {
    let rule = match gate(rolls.canon, rolls.snapshot, intent) {
        Ok(rule) => rule.clone(),
        Err(refusal) => return Ok(refused(refusal, None)),
    };
    if crate::map::span(rolls.canon, rolls.snapshot, intent, &rule) > 0 {
        return Ok(Resolution::Ready { rolled: None });
    }
    if let Some(roll) = rule.roll.clone() {
        rolls.log.steps.push("resolve_check".to_string());
        return roll_check(rolls, intent, &roll);
    }
    Ok(Resolution::Ready { rolled: None })
}

fn roll_check(
    rolls: &mut Rolls<'_>,
    intent: &Intent,
    roll: &crate::rule::RollSpec,
) -> Result<Resolution, Reason> {
    if let Err(code) = ViewerId::from_kind(intent.actor_kind, &intent.actor_id) {
        return Ok(refused_check(intent, code));
    }
    let opposed = match supplied_target(rolls, intent) {
        Ok(total) => total,
        Err(code) => return Ok(refused_check(intent, code)),
    };
    let (canon, snapshot) = (rolls.canon, rolls.snapshot);
    let (target, partial_at) = {
        let mut roller = rolls.dice.roller(
            rolls.log,
            &intent.move_name,
            &intent.actor_id,
            ResistKey::Rule,
        );
        let ctx = Ctx { intent };
        let partial_at = if opposed.is_some() {
            None
        } else {
            match roll.partial_at.as_ref() {
                Some(expr) => {
                    Some(expr::eval_with_dice(canon, snapshot, &ctx, expr, &mut roller)?.value)
                }
                None => None,
            }
        };
        let target = match opposed {
            Some(total) => total,
            None => expr::eval_with_dice(canon, snapshot, &ctx, &roll.target, &mut roller)?.value,
        };
        (target, partial_at)
    };
    if let Some(out) = rolls.opposed.get(&intent.id).cloned() {
        return Ok(Resolution::Ready {
            rolled: Some(reclassify(&out, target, partial_at)),
        });
    }
    let key = rolls.dice.key(&intent.move_name, &intent.actor_id);
    match rolls
        .dice
        .check(canon, snapshot, key, &roll.check, target, partial_at)
    {
        Ok(out) => {
            rolls.opposed.insert(intent.id.clone(), out.clone());
            Ok(Resolution::Ready { rolled: Some(out) })
        }
        Err(code) => Ok(refused_check(intent, code)),
    }
}

fn supplied_target(rolls: &mut Rolls<'_>, intent: &Intent) -> Result<Option<i64>, Reason> {
    let Target::Actor(named) = &intent.target else {
        return Ok(None);
    };
    let (canon, snapshot, pool) = (rolls.canon, rolls.snapshot, rolls.pool);
    let Some(opposing) = pool.iter().find(|other| {
        other.id != intent.id
            && &other.actor_id == named
            && other.source == Source::Player
            && gate(canon, snapshot, other)
                .ok()
                .is_some_and(|rule| rule.roll.is_some())
    }) else {
        return Ok(None);
    };
    if let Some(out) = rolls.opposed.get(&opposing.id) {
        return Ok(Some(out.total));
    }
    let Some(check_id) = gate(canon, snapshot, opposing)
        .ok()
        .and_then(|rule| rule.roll.as_ref().map(|roll| roll.check.clone()))
    else {
        return Ok(None);
    };
    let key = rolls.dice.key(&opposing.move_name, &opposing.actor_id);
    let out = rolls.dice.check(canon, snapshot, key, &check_id, 0, None)?;
    let total = out.total;
    rolls.opposed.insert(opposing.id.clone(), out);
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
