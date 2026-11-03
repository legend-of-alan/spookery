//! Finishes due pending actions, journeys arriving and timed actions completing.

use crate::canon::Canon;
use crate::draft::Draft;
use crate::intent::{Intent, Target};
use crate::model::{PendingAction, PendingKind};
use crate::rules::context::Context;
use crate::rules::effect::{Effect, EffectKind};
use crate::rules::expand::{self, Cause};
use crate::rules::gate;
use crate::store::records::Arrival;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;
use crate::round::stage::apply;

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
        let PendingAction { verb, source, kind } = due_action.pending;
        let actor_id = &due_action.actor_id;
        match kind {
            PendingKind::Arrive { place, via } => {
                let intent =
                    Intent::arrival(draft.state(), actor_id, source, &verb, Target::Empty)?;
                arrive(log, draft, &intent, due_action.until, place, via)?;
            }
            PendingKind::Finish { target, rule } => {
                let intent = Intent::arrival(draft.state(), actor_id, source, &verb, target)?;
                finish_action(draft, canon, &intent, &rule, dice, log)?;
            }
        }
    }
    Ok(())
}

fn arrive(
    log: &mut RoundLog,
    draft: &mut Draft,
    intent: &Intent,
    until: u64,
    to: String,
    via: String,
) -> Result<(), Reason> {
    let actor_id = intent.actor.actor_id();
    let actor = draft
        .state()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownId)?;
    let from = actor.place.clone();
    let effect = Effect::new(
        &intent.actor,
        &intent.id,
        EffectKind::Location {
            actor_id: actor_id.to_string(),
            from,
            to: to.clone(),
            arrived_at: Some(until),
        },
    );
    effect.clone().apply(draft, intent)?;
    log.effects.push(effect);
    log.arrivals.push(Arrival {
        actor_id: actor_id.to_string(),
        arrived_at: until,
        to,
        via,
    });
    Ok(())
}

/// Roll and expand the timed action's rule against the draft, then land it. A refusal is
/// journaled and the round goes on; the actor is no longer busy either way.
fn finish_action(
    draft: &mut Draft,
    canon: &Canon,
    intent: &Intent,
    rule_id: &str,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<(), Reason> {
    let rule = canon.rules.get(rule_id).ok_or(Reason::UnknownMove)?;
    let context = Context::new(canon, draft.state(), draft.presence, intent);
    let roller = dice.roller(log, &rule.id, intent.actor.actor_id());
    match expand::expand(&context, rule, Cause::Finish, None, roller) {
        Ok(expansion) => {
            apply::land(draft, intent, rule, expansion, log);
        }
        Err(code) => apply::refuse(log, intent, gate::Refusal::new(intent, code), false),
    }
    Ok(())
}
