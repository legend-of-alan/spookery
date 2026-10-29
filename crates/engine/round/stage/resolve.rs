//! Gate each intent and roll the checks it asks for.

use std::collections::BTreeMap;

use crate::check::classify;
use crate::rules::gate::{gate, GateRefusal};

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target};
use crate::journal::Step;
use crate::model::Now;
use crate::rules::Roll;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;

/// One candidate and how resolve left it for apply.
pub(crate) struct Resolved {
    pub intent: Intent,
    pub resolution: Resolution,
}

/// How resolve left an intent for apply.
pub(crate) enum Resolution {
    /// Gated; apply pays for and expands `rule_id`, the rule the gate chose against the snapshot.
    Ready {
        rule_id: String,
        rolled: Option<Roll>,
    },
    /// Refused by the gate: journaled, and told to a player.
    Refused(GateRefusal),
    /// The check could not be set up (no check or meter): journaled, and told to the actor whoever it is.
    CannotRoll(Reason),
}

/// What every roll in the round shares.
struct Rolls<'a> {
    canon: &'a Canon,
    snapshot: &'a Now,
    queue: &'a [Intent],
    opposed: BTreeMap<String, Roll>,
    dice: &'a mut RoundDice,
    log: &'a mut RoundLog,
}

pub(crate) fn run(
    canon: &Canon,
    snapshot: &Now,
    candidates: Vec<Intent>,
    queue: &[Intent],
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<Vec<Resolved>, Reason> {
    let mut rolls = Rolls {
        canon,
        snapshot,
        queue,
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
            rolls.log.steps.push(Step::CheckRolled);
            roll_intent_check(rolls, intent, rule, roll)
        }
        _ => Ok(Resolution::Ready {
            rule_id: rule.id.clone(),
            rolled: None,
        }),
    }
}

fn roll_intent_check(
    rolls: &mut Rolls<'_>,
    intent: &Intent,
    rule: &crate::rule::Rule,
    roll: &crate::rule::RollSpec,
) -> Result<Resolution, Reason> {
    let actor_id = intent.actor.actor_id();
    let opposed = match opposing_total(rolls, intent) {
        Ok(total) => total,
        Err(code) => return Ok(Resolution::CannotRoll(code)),
    };
    let (canon, snapshot) = (rolls.canon, rolls.snapshot);
    let (target, partial_at) = match opposed {
        Some(total) => (total, None),
        None => rolls
            .dice
            .roller(rolls.log, &rule.id, actor_id)
            .thresholds(canon, snapshot, intent, roll)?,
    };
    let rule_id = rule.id.clone();
    if let Some(out) = rolls.opposed.get(&intent.id) {
        return Ok(Resolution::Ready {
            rule_id,
            rolled: Some(rescore(out, target, partial_at)),
        });
    }
    let key = rolls.dice.key(&rule.id, actor_id);
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
    let (canon, snapshot, queue) = (rolls.canon, rolls.snapshot, rolls.queue);
    let Some((opposing, rule_id, check_id)) = queue.iter().find_map(|other| {
        if other.id == intent.id
            || other.actor.actor_id() != named
            || other.source != Source::Player
        {
            return None;
        }
        let rule = gate(canon, snapshot, other).ok()?;
        Some((other, &rule.id, &rule.roll.as_ref()?.check))
    }) else {
        return Ok(None);
    };
    if let Some(out) = rolls.opposed.get(&opposing.id) {
        return Ok(Some(out.total));
    }
    let key = rolls.dice.key(rule_id, opposing.actor.actor_id());
    let out = rolls.dice.check(canon, snapshot, key, check_id, 0, None)?;
    let total = out.total;
    rolls.opposed.insert(opposing.id.clone(), out);
    Ok(Some(total))
}

fn rescore(out: &Roll, target: i64, partial_at: Option<i64>) -> Roll {
    Roll {
        faces: out.faces.clone(),
        stat: out.stat,
        total: out.total,
        target,
        outcome: classify(out.total, target, partial_at),
    }
}
