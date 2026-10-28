//! Land journeys whose minute this tick has reached.

use crate::apply::apply_effect;
use crate::draft::Draft;
use crate::effect::{Effect, EffectKind};
use crate::expand::expand_outcome;
use crate::intent::{ActorKind, Intent, IntentArgs, Target, ViewerId};
use crate::journal::Arrival;
use crate::model::PendingAction;
use crate::rules::MoveReport;
use crate::Reason;

use super::resolve::{eval_roll_spec, resist_outcome};
use crate::round::state::Round;

/// Apply every journey whose minute has been reached, earliest first.
pub(crate) fn finish(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    let due_actions = crate::map::take_due_actions(draft, round.world_time_after)?;
    for due_action in due_actions {
        if due_action.pending.arrive.is_some() {
            arrive(
                round,
                draft,
                &due_action.actor_id,
                due_action.until,
                &due_action.pending,
            )?;
        } else {
            resume(round, draft, &due_action.actor_id, &due_action.pending)?;
        }
    }
    Ok(())
}

fn arrive(
    round: &mut Round,
    draft: &mut Draft,
    actor_id: &str,
    until: u64,
    pending: &PendingAction,
) -> Result<(), Reason> {
    let actor = draft.now().actors.get(actor_id).ok_or(Reason::UnknownKey)?;
    let from = actor.place().to_string();
    let to = pending.arrive.clone().unwrap_or_default();
    let kind = actor.actor_kind();
    let intent = arrival_intent(round, actor_id, kind, pending);
    let effect = Effect {
        class: EffectKind::Location {
            actor_id: actor_id.to_string(),
            from,
            to: to.clone(),
            arrived_at: Some(until),
        },
        as_viewer: ViewerId::from_kind(kind, actor_id)?,
        intent_id: intent.id.clone(),
    };
    apply_effect(draft, &intent, effect.clone())?;
    round.effects.push(effect);
    round.arrivals.push(Arrival {
        actor_id: actor_id.to_string(),
        arrived_at: until,
        to,
        via: pending.via.clone().unwrap_or_default(),
    });
    Ok(())
}

fn resume(
    round: &mut Round,
    draft: &mut Draft,
    actor_id: &str,
    pending: &PendingAction,
) -> Result<(), Reason> {
    let kind = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .actor_kind();
    let intent = arrival_intent(round, actor_id, kind, pending);
    let roll_spec = crate::rules::gate::resolve(draft.canon(), draft.now(), &intent)?
        .roll
        .clone();
    let canon = draft.canon().clone();
    let snapshot = draft.now().clone();
    let (effects, reports) = super::resolve::with_dice(round, |tracker| {
        let mut roller = tracker.roller(&intent.move_name, actor_id);
        let mut reports = Vec::new();
        let outcome = match &roll_spec {
            Some(spec) => {
                let out = eval_roll_spec(
                    &canon,
                    &snapshot,
                    &intent,
                    spec,
                    &spec.check,
                    tracker,
                    &mut roller,
                )?;
                let band = out.outcome;
                reports.push(MoveReport::Rolled {
                    actor_id: actor_id.to_string(),
                    out,
                });
                Some(band)
            }
            None => None,
        };
        let effects = expand_outcome(
            &canon,
            &snapshot,
            &intent,
            outcome,
            &mut roller,
            |who, check_id, target| {
                let (band, report) =
                    resist_outcome(&canon, &snapshot, check_id, who, check_id, target, tracker)?;
                reports.push(report);
                Ok(band)
            },
        )?;
        Ok::<_, Reason>((effects, reports))
    })?;
    round.move_reports.extend(reports);
    for effect in effects {
        if matches!(effect.class, EffectKind::Location { .. }) {
            continue;
        }
        apply_effect(draft, &intent, effect.clone())?;
        round.effects.push(effect);
    }
    Ok(())
}

fn arrival_intent(
    round: &Round,
    actor_id: &str,
    kind: ActorKind,
    pending: &PendingAction,
) -> Intent {
    Intent {
        id: format!("arrival:{actor_id}"),
        round_index: round.round_index,
        actor_id: actor_id.to_string(),
        actor_kind: kind,
        source: pending.source,
        move_name: pending.verb.clone(),
        target: if pending.target.is_empty() {
            Target::Empty
        } else {
            Target::Place(pending.target.clone())
        },
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}
