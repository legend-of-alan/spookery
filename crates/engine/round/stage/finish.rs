//! Finish each timed action whose ticks have passed, a walk's arrival among them, and run the
//! trigger pass after each.

use crate::canon::Canon;
use crate::draft::Draft;
use crate::intent::Intent;
use crate::model::PendingAction;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;
use crate::round::stage::{apply, triggers};

/// Finish every due action, earliest first.
pub(crate) fn run(
    draft: &mut Draft,
    canon: &Canon,
    end: u64,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<(), Reason> {
    let due_actions = crate::rules::travel::take_due_actions(draft, end)?;
    for due_action in due_actions {
        let PendingAction {
            verb,
            source,
            target,
            rule,
        } = due_action.pending;
        let actor_id = &due_action.actor_id;
        let intent = Intent::finish(draft.state(), actor_id, source, &verb, target)?;
        let rule = canon.rules.get(&rule).ok_or(Reason::UnknownMove)?;
        let until = due_action.until;
        apply::finish(draft, canon, &intent, rule, None, until, dice, log);
        triggers::run(draft, canon, dice, log);
    }
    Ok(())
}
