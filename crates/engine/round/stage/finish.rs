//! Finishes due pending actions, journeys arriving and timed actions completing.

use crate::draft::Draft;
use crate::effect::{Effect, EffectKind};
use crate::intent::{Intent, Target};
use crate::journal::Arrival;
use crate::model::{PendingAction, PendingKind};
use crate::rules::case::Case;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;

/// Finish every due action, earliest first.
pub(crate) fn run(
    draft: &mut Draft,
    end: u64,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<(), Reason> {
    let due_actions = crate::travel::take_due_actions(draft, end)?;
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
                finish_timed_action(dice, log, draft, &intent, &rule)?;
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
    let from = actor.place().to_string();
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

fn finish_timed_action(
    dice: &mut RoundDice,
    log: &mut RoundLog,
    draft: &mut Draft,
    intent: &Intent,
    rule_id: &str,
) -> Result<(), Reason> {
    let actor_id = intent.actor.actor_id();
    let case = Case::new(draft.canon(), draft.state(), draft.presence, intent);
    let rule = case.canon.rules.get(rule_id).ok_or(Reason::UnknownMove)?;
    let mut roller = dice.roller(log, &rule.id, actor_id);
    let rolled = match &rule.roll {
        Some(spec) => Some(roller.roll_spec(&case, spec)?),
        None => None,
    };
    let outcome = rolled.as_ref().map(|out| out.outcome);
    let effects = rule.expand(&case, outcome, &mut roller)?;
    let resisted = roller.into_reports();
    log.report_roll(rolled, actor_id);
    log.reports.extend(resisted);
    for effect in effects {
        if matches!(effect.kind, EffectKind::Location { .. }) {
            continue;
        }
        effect.clone().apply(draft, intent)?;
        log.effects.push(effect);
    }
    Ok(())
}
