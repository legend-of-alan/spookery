//! Land journeys whose minute this tick has reached.

use spookery_kernel::api::apply::apply_effect;
use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::effect::{Effect, EffectKind};
use spookery_kernel::api::expand::expand_outcome;
use spookery_kernel::api::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::api::journal::Arrival;
use spookery_kernel::api::model::Pending;
use spookery_kernel::api::rules::MoveReport;
use spookery_kernel::api::Reason;

use super::resolve::{eval_roll_spec, resist_outcome};
use crate::state::Round;

/// The stage mark. Arrivals land in [`finish`], after this tick's intents,
/// so a traveller acts again on the next tick.
pub(crate) fn run(round: &mut Round, _draft: &mut Draft) -> Result<(), Reason> {
    round.steps_run.push("complete".to_string());
    Ok(())
}

/// Apply every journey whose minute has been reached, earliest first.
pub(crate) fn finish(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    let due = spookery_kernel::api::map::take_due(draft, round.world_time_after)?;
    for item in due {
        if item.pending.arrive.is_some() {
            arrive(round, draft, &item.actor_id, item.until, &item.pending)?;
        } else {
            resume(round, draft, &item.actor_id, &item.pending)?;
        }
    }
    Ok(())
}

fn arrive(
    round: &mut Round,
    draft: &mut Draft,
    actor_id: &str,
    until: u64,
    pending: &Pending,
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
    pending: &Pending,
) -> Result<(), Reason> {
    let kind = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .actor_kind();
    let intent = arrival_intent(round, actor_id, kind, pending);
    let roll_spec =
        spookery_kernel::api::rules::gate::resolve(draft.canon(), draft.now(), &intent)?
            .roll
            .clone();
    let canon = draft.canon().clone();
    let snapshot = draft.now().clone();
    let (effects, reports) = super::resolve::with_dice(round, |held| {
        let mut roller = held.roller(&intent.move_name, actor_id);
        let mut reports = Vec::new();
        let outcome = match &roll_spec {
            Some(spec) => {
                let out = eval_roll_spec(
                    &canon,
                    &snapshot,
                    &intent,
                    spec,
                    &spec.check,
                    held,
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
                    resist_outcome(&canon, &snapshot, check_id, who, check_id, target, held)?;
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

fn arrival_intent(round: &Round, actor_id: &str, kind: ActorKind, pending: &Pending) -> Intent {
    Intent {
        id: format!("arrival:{actor_id}"),
        tick_index: round.tick_index,
        actor_id: actor_id.to_string(),
        actor_kind: kind,
        source: match pending.source.as_str() {
            "kernel" => Source::Kernel,
            "system" => Source::System,
            _ => Source::Player,
        },
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
