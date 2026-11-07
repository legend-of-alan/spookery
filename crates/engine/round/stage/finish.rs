//! Finish each timed action whose ticks have passed, a walk's arrival among them, and run the
//! trigger pass after each.

use crate::canon::Canon;
use crate::intent::Intent;
use crate::model::PendingAction;
use crate::model::State;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;
use crate::round::stage::{apply, triggers};

/// Finish every due action, earliest first.
pub(crate) fn run(
    canon: &Canon,
    state: &mut State,
    end: u64,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<(), Reason> {
    let due_actions = crate::rules::travel::take_due_actions(state, end)?;
    for due_action in due_actions {
        let PendingAction {
            verb,
            source,
            target,
            rule,
        } = due_action.pending;
        let actor_id = &due_action.actor_id;
        let intent = Intent::finish(state, actor_id, source, &verb, target)?;
        let rule = canon.rules.get(&rule).ok_or(Reason::UnknownMove)?;
        let until = due_action.until;
        apply::finish(canon, state, &intent, rule, None, until, dice, log);
        triggers::run(canon, state, dice, log);
    }
    Ok(())
}
