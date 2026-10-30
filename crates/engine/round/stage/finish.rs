//! Finishes due pending actions, journeys arriving and timed actions completing.

use crate::draft::Draft;
use crate::effect::{Effect, EffectKind};
use crate::intent::{due_intent_id, Intent, Source, Target};
use crate::journal::Arrival;
use crate::model::{PendingAction, PendingKind};
use crate::rules::MoveReport;
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
                let intent = due_intent(draft, actor_id, source, &verb, Target::Empty)?;
                arrive(log, draft, &intent, due_action.until, place, via)?;
            }
            PendingKind::Finish { target, rule } => {
                let intent = due_intent(draft, actor_id, source, &verb, target)?;
                finish_timed_action(dice, log, draft, &intent, &rule)?;
            }
        }
    }
    Ok(())
}

fn due_intent(
    draft: &Draft,
    actor_id: &str,
    source: Source,
    verb: &str,
    target: Target,
) -> Result<Intent, Reason> {
    let actor = draft
        .state()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?;
    Ok(Intent::new(
        due_intent_id(actor_id),
        draft.state().round_index,
        actor.viewer(actor_id),
        source,
        verb,
        target,
    ))
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
        .ok_or(Reason::UnknownKey)?;
    let from = actor.place().to_string();
    let effect = Effect {
        class: EffectKind::Location {
            actor_id: actor_id.to_string(),
            from,
            to: to.clone(),
            arrived_at: Some(until),
        },
        viewer: intent.actor.clone(),
        intent_id: intent.id.clone(),
    };
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
    let (canon, snapshot, presence) = (draft.canon(), draft.state(), draft.presence);
    let rule = canon.rules.get(rule_id).ok_or(Reason::UnknownMove)?;
    let mut roller = dice.roller(log, &rule.id, actor_id);
    let rolled = match &rule.roll {
        Some(spec) => Some(roller.roll_spec(canon, snapshot, presence, intent, spec)?),
        None => None,
    };
    let outcome = rolled.as_ref().map(|out| out.outcome);
    let effects = rule.expand(canon, snapshot, presence, intent, outcome, &mut roller)?;
    let resisted = roller.into_reports();
    let actor_id = actor_id.to_string();
    log.reports.extend(rolled.map(|out| MoveReport::Rolled {
        actor_id,
        roll: out,
    }));
    log.reports.extend(resisted);
    for effect in effects {
        if matches!(effect.class, EffectKind::Location { .. }) {
            continue;
        }
        effect.clone().apply(draft, intent)?;
        log.effects.push(effect);
    }
    Ok(())
}
