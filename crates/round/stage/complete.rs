use spookery_kernel::api::apply::apply_effect;
use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::effect::{Effect, EffectKind};
use spookery_kernel::api::expand::expand_with;
use spookery_kernel::api::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::api::journal::Arrival;
use spookery_kernel::api::model::Pending;
use spookery_kernel::api::rules::expr::{self, Ctx};
use spookery_kernel::api::rules::RollReport;
use spookery_kernel::api::Reason;

use super::resolve::{roll_with_key, SubjectRoll};
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
    let kind = kind_of(actor.is_pc());
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
    let is_pc = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .is_pc();
    let kind = kind_of(is_pc);
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
        let ctx = Ctx { intent: &intent };
        let outcome = match &roll_spec {
            Some(spec) => {
                let target = expr::show(&canon, &snapshot, &ctx, &spec.target, &mut roller)?.value;
                let partial_at = match &spec.partial_at {
                    Some(expr) => {
                        Some(expr::show(&canon, &snapshot, &ctx, expr, &mut roller)?.value)
                    }
                    None => None,
                };
                let actor = snapshot
                    .actors
                    .get(actor_id)
                    .ok_or(Reason::UnknownKey)?
                    .clone();
                let key = held.key(&spec.check, actor_id);
                let out = roll_with_key(
                    &snapshot,
                    &canon.checks,
                    &canon.meters,
                    &SubjectRoll {
                        rule_id: &spec.check,
                        actor_kind: kind,
                        actor_id,
                        check_id: &spec.check,
                        target,
                        partial_at,
                        live: Some(&actor),
                    },
                    &key,
                )?;
                let band = out.outcome;
                reports.push(RollReport::Rolled {
                    actor_id: actor_id.to_string(),
                    out,
                });
                Some(band)
            }
            None => None,
        };
        let effects = expand_with(
            &canon,
            &snapshot,
            &intent,
            outcome,
            &mut roller,
            |who, check_id, target| {
                let who_pc = snapshot.actors.get(who).ok_or(Reason::UnknownKey)?.is_pc();
                let who_kind = kind_of(who_pc);
                let actor = snapshot.actors.get(who).ok_or(Reason::UnknownKey)?.clone();
                let key = held.key(check_id, who);
                let out = roll_with_key(
                    &snapshot,
                    &canon.checks,
                    &canon.meters,
                    &SubjectRoll {
                        rule_id: check_id,
                        actor_kind: who_kind,
                        actor_id: who,
                        check_id,
                        target,
                        partial_at: None,
                        live: Some(&actor),
                    },
                    &key,
                )?;
                let band = out.outcome;
                reports.push(RollReport::Rolled {
                    actor_id: who.to_string(),
                    out,
                });
                Ok(band)
            },
        )?;
        Ok::<_, Reason>((effects, reports))
    })?;
    round.roll_reports.extend(reports);
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

fn kind_of(is_pc: bool) -> ActorKind {
    if is_pc {
        ActorKind::Player
    } else {
        ActorKind::Npc
    }
}
