//! Pick each intent's rule. The gate runs at queue and start; the check rolls when the move lands.

use crate::rules::context::Context;
use crate::rules::gate;

use crate::canon::Canon;
use crate::ecs::{self, Presence};
use crate::intent::Intent;
use crate::rules::rule::Rule;

/// One candidate and how resolve left it for apply.
pub(crate) struct Candidate<'a> {
    pub intent: &'a Intent,
    pub resolution: Resolution<'a>,
}

/// How resolve left an intent for apply.
pub(crate) enum Resolution<'a> {
    /// Picked against the state. Apply admits `rule` again against the working copy when it starts,
    /// then pays for and expands it, so a move pays the cost of the rule it picked.
    Ready { rule: &'a Rule },
    /// A player's `cancel`, which runs no rule.
    Cancel,
    /// No rule picked: journaled, and told to a player.
    Refused(gate::Refusal),
}

pub(crate) fn run<'a>(
    canon: &'a Canon,
    world: &ecs::World,
    presence: &Presence,
    picked: &'a [Intent],
) -> Vec<Candidate<'a>> {
    picked
        .iter()
        .map(|intent| Candidate {
            intent,
            resolution: resolve(canon, world, presence, intent),
        })
        .collect()
}

fn resolve<'a>(
    canon: &'a Canon,
    world: &ecs::World,
    presence: &Presence,
    intent: &'a Intent,
) -> Resolution<'a> {
    if gate::is_cancel(intent) {
        return Resolution::Cancel;
    }
    match gate::rule_of(&Context::new(canon, world, presence, intent)) {
        Ok(rule) => Resolution::Ready { rule },
        Err(refusal) => Resolution::Refused(refusal),
    }
}
