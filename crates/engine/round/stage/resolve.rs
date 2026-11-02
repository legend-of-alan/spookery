//! Gate each intent and roll the checks it asks for.

use std::collections::BTreeMap;

use crate::rules::context::Context;
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
    /// Gated against the state. Apply admits `rule` again against the draft, then pays for
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
    state: &'a State,
    presence: &'a Presence,
    queue: &'a [Intent],
    rolled: BTreeMap<String, Roll>,
    dice: &'r mut RoundDice,
    log: &'r mut RoundLog,
}

pub(crate) fn run<'a>(
    canon: &'a Canon,
    state: &'a State,
    presence: &'a Presence,
    picked: &'a [Intent],
    queue: &'a [Intent],
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<Vec<Candidate<'a>>, Reason> {
    let mut resolver = Resolver {
        canon,
        state,
        presence,
        queue,
        rolled: BTreeMap::new(),
        dice,
        log,
    };
    let mut candidates = Vec::with_capacity(picked.len());
    for intent in picked {
        let resolution = gate_intent(&mut resolver, intent)?;
        candidates.push(Candidate { intent, resolution });
    }
    Ok(candidates)
}

fn gate_intent<'a>(
    resolver: &mut Resolver<'a, '_>,
    intent: &'a Intent,
) -> Result<Resolution<'a>, Reason> {
    let context = Context::new(resolver.canon, resolver.state, resolver.presence, intent);
    let rule = match gate::gate(&context) {
        Ok(rule) => rule,
        Err(refusal) => return Ok(Resolution::Refused(refusal)),
    };
    match &rule.roll {
        Some(roll) if crate::rules::travel::span(&context, rule) == 0 => {
            resolver.log.extra_steps.push(Step::CheckRolled);
            roll_check(resolver, intent, rule, roll)
        }
        _ => Ok(Resolution::Ready { rule, rolled: None }),
    }
}

/// Roll the intent's check, against the opposing total when another player's move opposes it.
/// An intent already rolled as another's opposition keeps its faces and takes its own target.
fn roll_check<'a>(
    resolver: &mut Resolver,
    intent: &Intent,
    rule: &'a Rule,
    roll: &RollSpec,
) -> Result<Resolution<'a>, Reason> {
    let actor_id = intent.actor.actor_id();
    let opposed = match opposing_total(resolver, intent) {
        Ok(total) => total,
        Err(code) => return Ok(Resolution::CannotRoll(code)),
    };
    let (canon, state) = (resolver.canon, resolver.state);
    let (target, partial_at) = match opposed {
        Some(total) => (total, None),
        None => resolver
            .dice
            .roller(resolver.log, &rule.id, actor_id)
            .eval_thresholds(&Context::new(canon, state, resolver.presence, intent), roll)?,
    };
    if let Some(out) = resolver.rolled.get(&intent.id) {
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
    let key = resolver.dice.next_key(&rule.id, actor_id);
    match resolver
        .dice
        .roll_check(canon, state, key, &roll.check, target, partial_at)
    {
        Ok(out) => {
            resolver.rolled.insert(intent.id.clone(), out.clone());
            Ok(Resolution::Ready {
                rule,
                rolled: Some(out),
            })
        }
        Err(code) => Ok(Resolution::CannotRoll(code)),
    }
}

fn opposing_total(resolver: &mut Resolver, intent: &Intent) -> Result<Option<i64>, Reason> {
    let Target::Actor(named) = &intent.target else {
        return Ok(None);
    };
    let (canon, state, presence, queue) = (
        resolver.canon,
        resolver.state,
        resolver.presence,
        resolver.queue,
    );
    let Some((opposing, rule_id, check_id)) = queue.iter().find_map(|other| {
        if other.id == intent.id
            || other.actor.actor_id() != named
            || other.source != Source::Player
        {
            return None;
        }
        let rule = gate::gate(&Context::new(canon, state, presence, other)).ok()?;
        Some((other, &rule.id, &rule.roll.as_ref()?.check))
    }) else {
        return Ok(None);
    };
    if let Some(out) = resolver.rolled.get(&opposing.id) {
        return Ok(Some(out.total));
    }
    let key = resolver.dice.next_key(rule_id, opposing.actor.actor_id());
    let out = resolver
        .dice
        .roll_check(canon, state, key, check_id, 0, None)?;
    let total = out.total;
    resolver.rolled.insert(opposing.id.clone(), out);
    Ok(Some(total))
}
