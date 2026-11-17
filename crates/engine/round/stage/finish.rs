//! Finish each timed action whose ticks have passed, a walk's arrival among them, and run the
//! trigger pass after each.

use crate::canon::Canon;
use crate::ecs::{self, PendingAction, Presence};
use crate::intent::Intent;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::stage::{apply, triggers};
use crate::round::Ledger;

/// Finish every due action, earliest first.
pub(crate) fn run(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    end: u64,
    dice: &mut RoundDice,
    ledger: &mut Ledger,
) -> Result<(), Reason> {
    let due_actions = crate::rules::travel::take_due_actions(world, end)?;
    for due_action in due_actions {
        let PendingAction {
            verb,
            source,
            target,
            rule,
        } = due_action.pending;
        let actor_id = &due_action.actor_id;
        let intent = Intent::finish(world, actor_id, source, &verb, target)?;
        let rule = canon.rules.get(&rule).ok_or(Reason::UnknownMove)?;
        let until = due_action.until;
        apply::finish(
            canon, world, presence, &intent, rule, None, until, dice, ledger,
        );
        triggers::run(canon, world, presence, dice, ledger);
    }
    Ok(())
}
