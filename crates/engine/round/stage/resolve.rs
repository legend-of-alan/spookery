//! Gate each intent and roll the checks it asks for.

use std::collections::BTreeMap;

use crate::check::classify;
use crate::rules::expr::{self, Ctx};
use crate::rules::gate::{gate, GateRefusal};

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target, ViewerId};
use crate::model::Now;
use crate::rules::RollResult;
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
    /// Gated; apply expands `rule_id`, the rule the gate chose against the snapshot.
    Ready {
        rule_id: String,
        rolled: Option<RollResult>,
    },
    /// Refused by the gate: journaled, and told to a player.
    Refused(GateRefusal),
    /// The check could not be set up (no viewer, check or meter): journaled, and told to the actor whoever it is.
    CannotRoll(Reason),
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
        let resolution = gate_and_roll(&mut rolls, &intent)?;
        resolved.push(Resolved { intent, resolution });
    }
    Ok(resolved)
}

fn gate_and_roll(rolls: &mut Rolls<'_>, intent: &Intent) -> Result<Resolution, Reason> {
    let (canon, snapshot) = (rolls.canon, rolls.snapshot);
    let rule = match gate(canon, snapshot, intent) {
        Ok(rule) => rule,
        Err(refusal) => return Ok(Resolution::Refused(refusal)),
    };
    match &rule.roll {
        Some(roll) if crate::map::span(canon, snapshot, intent, rule) == 0 => {
            rolls.log.steps.push("resolve_check".to_string());
            roll_check(rolls, intent, rule, roll)
        }
        _ => Ok(Resolution::Ready {
            rule_id: rule.id.clone(),
            rolled: None,
        }),
    }
}

fn roll_check(
    rolls: &mut Rolls<'_>,
    intent: &Intent,
    rule: &crate::rule::Rule,
    roll: &crate::rule::RollSpec,
) -> Result<Resolution, Reason> {
    if let Err(code) = ViewerId::from_kind(intent.actor_kind, &intent.actor_id) {
        return Ok(Resolution::CannotRoll(code));
    }
    let opposed = match opposing_total(rolls, intent) {
        Ok(total) => total,
        Err(code) => return Ok(Resolution::CannotRoll(code)),
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
    let rule_id = rule.id.clone();
    if let Some(out) = rolls.opposed.get(&intent.id) {
        return Ok(Resolution::Ready {
            rule_id,
            rolled: Some(reclassify(out, target, partial_at)),
        });
    }
    let key = rolls.dice.key(&intent.move_name, &intent.actor_id);
    match rolls
        .dice
        .check(canon, snapshot, key, &roll.check, target, partial_at)
    {
        Ok(out) => {
            rolls.opposed.insert(intent.id.clone(), out.clone());
            Ok(Resolution::Ready {
                rule_id,
                rolled: Some(out),
            })
        }
        Err(code) => Ok(Resolution::CannotRoll(code)),
    }
}

fn opposing_total(rolls: &mut Rolls<'_>, intent: &Intent) -> Result<Option<i64>, Reason> {
    let Target::Actor(named) = &intent.target else {
        return Ok(None);
    };
    let (canon, snapshot, pool) = (rolls.canon, rolls.snapshot, rolls.pool);
    let Some((opposing, check_id)) = pool.iter().find_map(|other| {
        if other.id == intent.id || &other.actor_id != named || other.source != Source::Player {
            return None;
        }
        let check_id = gate(canon, snapshot, other)
            .ok()?
            .roll
            .as_ref()?
            .check
            .clone();
        Some((other, check_id))
    }) else {
        return Ok(None);
    };
    if let Some(out) = rolls.opposed.get(&opposing.id) {
        return Ok(Some(out.total));
    }
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
