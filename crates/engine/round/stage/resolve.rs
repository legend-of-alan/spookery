//! Gate each intent and roll the checks it asks for.

use std::collections::BTreeMap;

use crate::rules::case::Case;
use crate::rules::gate;
use crate::rules::roll::classify;

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target};
use crate::model::{Presence, State};
use crate::rules::roll::Roll;
use crate::rules::rule::{RollSpec, Rule};
use crate::store::journal::Step;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;

/// One candidate and how resolve left it for apply.
pub(crate) struct Candidate<'a> {
    pub intent: &'a Intent,
    pub resolution: Resolution<'a>,
}

/// How resolve left an intent for apply.
pub(crate) enum Resolution<'a> {
    /// Gated against the snapshot. Apply admits `rule` again against the draft, then pays for
    /// and expands it, so a move pays the cost of the rule it rolled for.
    Ready {
        rule: &'a Rule,
        rolled: Option<Roll>,
    },
    /// Refused by the gate: journaled, and told to a player.
    Refused(gate::Refusal),
    /// The check could not be set up (no check or meter): journaled, and told to the actor whoever it is.
    CannotRoll(Reason),
}

/// What every roll in the round shares. The rules it gates borrow `canon`, past the resolver.
struct Resolver<'a, 'r> {
    canon: &'a Canon,
    snapshot: &'a State,
    presence: &'a Presence,
    queue: &'a [Intent],
    rolled: BTreeMap<String, Roll>,
    dice: &'r mut RoundDice,
    log: &'r mut RoundLog,
}

pub(crate) fn run<'a>(
    canon: &'a Canon,
    snapshot: &'a State,
    presence: &'a Presence,
    picked: &'a [Intent],
    queue: &'a [Intent],
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<Vec<Candidate<'a>>, Reason> {
    let mut rolls = Resolver {
        canon,
        snapshot,
        presence,
        queue,
        rolled: BTreeMap::new(),
        dice,
        log,
    };
    let mut candidates = Vec::with_capacity(picked.len());
    for intent in picked {
        let resolution = gate_and_roll(&mut rolls, intent)?;
        candidates.push(Candidate { intent, resolution });
    }
    Ok(candidates)
}

fn gate_and_roll<'a>(
    rolls: &mut Resolver<'a, '_>,
    intent: &'a Intent,
) -> Result<Resolution<'a>, Reason> {
    let case = Case::new(rolls.canon, rolls.snapshot, rolls.presence, intent);
    let rule = match gate::gate(&case) {
        Ok(rule) => rule,
        Err(refusal) => return Ok(Resolution::Refused(refusal)),
    };
    match &rule.roll {
        Some(roll) if crate::rules::travel::span(&case, rule) == 0 => {
            rolls.log.extra_steps.push(Step::CheckRolled);
            roll_intent_check(rolls, intent, rule, roll)
        }
        _ => Ok(Resolution::Ready { rule, rolled: None }),
    }
}

/// Roll the intent's check, against the opposing total when another player's move opposes it.
/// An intent already rolled as another's opposition keeps its faces and takes its own target.
fn roll_intent_check<'a>(
    rolls: &mut Resolver,
    intent: &Intent,
    rule: &'a Rule,
    roll: &RollSpec,
) -> Result<Resolution<'a>, Reason> {
    let actor_id = intent.actor.actor_id();
    let opposed = match roll_opposing_total(rolls, intent) {
        Ok(total) => total,
        Err(code) => return Ok(Resolution::CannotRoll(code)),
    };
    let (canon, snapshot) = (rolls.canon, rolls.snapshot);
    let (target, partial_at) = match opposed {
        Some(total) => (total, None),
        None => rolls
            .dice
            .roller(rolls.log, &rule.id, actor_id)
            .eval_thresholds(&Case::new(canon, snapshot, rolls.presence, intent), roll)?,
    };
    if let Some(out) = rolls.rolled.get(&intent.id) {
        let outcome = classify(out.total, target, partial_at);
        let rolled = Roll {
            target,
            outcome,
            ..out.clone()
        };
        return Ok(Resolution::Ready {
            rule,
            rolled: Some(rolled),
        });
    }
    let key = rolls.dice.next_key(&rule.id, actor_id);
    match rolls
        .dice
        .roll_check(canon, snapshot, key, &roll.check, target, partial_at)
    {
        Ok(out) => {
            rolls.rolled.insert(intent.id.clone(), out.clone());
            Ok(Resolution::Ready {
                rule,
                rolled: Some(out),
            })
        }
        Err(code) => Ok(Resolution::CannotRoll(code)),
    }
}

fn roll_opposing_total(rolls: &mut Resolver, intent: &Intent) -> Result<Option<i64>, Reason> {
    let Target::Actor(named) = &intent.target else {
        return Ok(None);
    };
    let (canon, snapshot, presence, queue) =
        (rolls.canon, rolls.snapshot, rolls.presence, rolls.queue);
    let Some((opposing, rule_id, check_id)) = queue.iter().find_map(|other| {
        if other.id == intent.id
            || other.actor.actor_id() != named
            || other.source != Source::Player
        {
            return None;
        }
        let rule = gate::gate(&Case::new(canon, snapshot, presence, other)).ok()?;
        Some((other, &rule.id, &rule.roll.as_ref()?.check))
    }) else {
        return Ok(None);
    };
    if let Some(out) = rolls.rolled.get(&opposing.id) {
        return Ok(Some(out.total));
    }
    let key = rolls.dice.next_key(rule_id, opposing.actor.actor_id());
    let out = rolls
        .dice
        .roll_check(canon, snapshot, key, check_id, 0, None)?;
    let total = out.total;
    rolls.rolled.insert(opposing.id.clone(), out);
    Ok(Some(total))
}
