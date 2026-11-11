//! Pick each intent's rule and roll the check it asks for. The gate runs at queue and start.

use std::collections::BTreeMap;

use crate::rules::context::Context;
use crate::rules::gate;
use crate::rules::roll::classify;

use crate::canon::Canon;
use crate::intent::{Intent, Target};
use crate::rules::roll::Roll;
use crate::rules::rule::{RollSpec, Rule, When};
use crate::state::{Presence, State};
use crate::store::records::Step;
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
    /// Picked against the state. Apply admits `rule` again against the working copy when it starts,
    /// then pays for and expands it, so a move pays the cost of the rule it rolled for.
    Ready {
        rule: &'a Rule,
        rolled: Option<Roll>,
    },
    /// A player's `cancel`, which runs no rule.
    Cancel,
    /// No rule picked: journaled, and told to a player.
    Refused(gate::Refusal),
    /// The check could not be set up (no check or meter, or a threshold that fails to evaluate):
    /// journaled, and told to the actor whoever it is.
    CannotRoll(Reason),
}

/// What every roll in the round shares. The rules it gates borrow `canon`, past the resolver
/// and the save.
struct Resolver<'c, 'a, 'r> {
    canon: &'c Canon,
    state: &'a State,
    presence: &'a Presence,
    queue: &'a [Intent],
    rolled: BTreeMap<String, Roll>,
    dice: &'r mut RoundDice,
    log: &'r mut RoundLog,
}

pub(crate) fn run<'c>(
    canon: &'c Canon,
    state: &State,
    presence: &Presence,
    picked: &'c [Intent],
    queue: &[Intent],
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Vec<Candidate<'c>> {
    let mut resolver = Resolver {
        canon,
        state,
        presence,
        queue,
        rolled: BTreeMap::new(),
        dice,
        log,
    };
    picked
        .iter()
        .map(|intent| Candidate {
            intent,
            resolution: resolve_intent(&mut resolver, intent),
        })
        .collect()
}

fn resolve_intent<'c>(resolver: &mut Resolver<'c, '_, '_>, intent: &'c Intent) -> Resolution<'c> {
    if gate::is_cancel(intent) {
        return Resolution::Cancel;
    }
    let context = Context::new(resolver.canon, resolver.state, resolver.presence, intent);
    let rule = match gate::rule_of(&context) {
        Ok(rule) => rule,
        Err(refusal) => return Resolution::Refused(refusal),
    };
    match &rule.roll {
        Some(roll) if crate::rules::travel::span(&context, rule) == 0 => {
            resolver.log.extra_steps.push(Step::CheckRolled);
            match roll_check(resolver, &context, &rule.id, roll) {
                Ok(rolled) => Resolution::Ready {
                    rule,
                    rolled: Some(rolled),
                },
                Err(code) => Resolution::CannotRoll(code),
            }
        }
        _ => Resolution::Ready { rule, rolled: None },
    }
}

/// Roll the intent's check, against the opposing total when another player's move opposes it.
/// An intent already rolled as another's opposition keeps its faces and takes its own target.
fn roll_check(
    resolver: &mut Resolver,
    context: &Context,
    rule_id: &str,
    roll: &RollSpec,
) -> Result<Roll, Reason> {
    let intent = context.intent;
    let actor_id = intent.actor_id();
    let opposed = opposing_total(resolver, intent)?;
    if let Some(out) = resolver.rolled.get(&intent.id) {
        let (target, partial_at) = match opposed {
            Some(total) => (total, None),
            None => resolver
                .dice
                .roller(resolver.log, rule_id, actor_id)
                .eval_thresholds(context, roll)?,
        };
        let outcome = classify(out.total, target, partial_at);
        return Ok(Roll {
            target,
            outcome,
            ..out.clone()
        });
    }
    let out = match opposed {
        Some(total) => {
            let key = resolver.dice.next_key(rule_id, actor_id);
            let (canon, state) = (resolver.canon, resolver.state);
            resolver
                .dice
                .roll_check(canon, state, key, &roll.check, total, None)?
        }
        None => resolver
            .dice
            .roller(resolver.log, rule_id, actor_id)
            .roll_spec(context, roll)?,
    };
    resolver.rolled.insert(intent.id.clone(), out.clone());
    Ok(out)
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
        if other.id == intent.id || other.actor_id() != Some(named) {
            return None;
        }
        let rule = gate::rule_of(&Context::new(canon, state, presence, other)).ok()?;
        // Only a verb opposes: a habit is never the actor's answer to a move.
        if !matches!(rule.when, When::Verb { .. }) {
            return None;
        }
        Some((other, &rule.id, &rule.roll.as_ref()?.check))
    }) else {
        return Ok(None);
    };
    if let Some(out) = resolver.rolled.get(&opposing.id) {
        return Ok(Some(out.total));
    }
    let key = resolver.dice.next_key(rule_id, opposing.actor_id());
    let out = resolver
        .dice
        .roll_check(canon, state, key, check_id, 0, None)?;
    let total = out.total;
    resolver.rolled.insert(opposing.id.clone(), out);
    Ok(Some(total))
}
