use spookery_kernel::apply::apply_effect;
use spookery_kernel::effect::{Effect, EffectClass};
use spookery_kernel::expand::expand_outcome;
use spookery_kernel::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::journal::Arrival;
use spookery_kernel::model::Pending;
use spookery_kernel::rules::expr::{self, Ctx};
use spookery_kernel::rules::RollReport;
use spookery_kernel::world::Draft;
use spookery_kernel::Reason;

use crate::resolve::{roll_subject, SubjectRoll};
use crate::round::Round;

/// The stage mark. Arrivals land in [`finish`], after this tick's intents,
/// so a traveller acts again on the next tick.
pub(crate) fn run(round: &mut Round, _draft: &mut Draft) -> Result<(), Reason> {
    round.trace.push("complete".to_string());
    Ok(())
}

/// Apply every journey whose minute has been reached, earliest first.
pub(crate) fn finish(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    let due = spookery_kernel::map::take_due(draft, round.world_time_after)?;
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
        class: EffectClass::Location {
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
    let roll_spec = spookery_kernel::rules::gate::resolve(draft.canon(), draft.now(), &intent)?
        .roll
        .clone();
    let outcome = match roll_spec {
        Some(spec) => {
            let ctx = Ctx { intent: &intent };
            let target = expr::eval(draft.canon(), draft.now(), &ctx, &spec.target)?;
            let partial_at = match &spec.partial_at {
                Some(expr) => Some(expr::eval(draft.canon(), draft.now(), &ctx, expr)?),
                None => None,
            };
            let out = live_roll(
                round,
                draft,
                actor_id,
                kind,
                &spec.check,
                target,
                partial_at,
            )?;
            let band = out.outcome;
            round.roll_reports.push(RollReport::Rolled {
                actor_id: actor_id.to_string(),
                out,
            });
            Some(band)
        }
        None => None,
    };
    let canon = draft.canon().clone();
    let snapshot = draft.now().clone();
    let effects = expand_outcome(
        &canon,
        &snapshot,
        &intent,
        outcome,
        |who, check_id, target| {
            let who_pc = draft
                .now()
                .actors
                .get(who)
                .ok_or(Reason::UnknownKey)?
                .is_pc();
            let who_kind = kind_of(who_pc);
            let out = live_roll(round, draft, who, who_kind, check_id, target, None)?;
            let band = out.outcome;
            round.roll_reports.push(RollReport::Rolled {
                actor_id: who.to_string(),
                out,
            });
            Ok(band)
        },
    )?;
    for effect in effects {
        if matches!(effect.class, EffectClass::Location { .. }) {
            continue;
        }
        apply_effect(draft, &intent, effect.clone())?;
        round.effects.push(effect);
    }
    Ok(())
}

fn live_roll(
    round: &mut Round,
    draft: &Draft,
    actor_id: &str,
    kind: ActorKind,
    check_id: &str,
    target: i64,
    partial_at: Option<i64>,
) -> Result<spookery_kernel::rules::RollResult, Reason> {
    let actor = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .clone();
    roll_subject(
        round,
        &draft.canon().checks,
        &draft.canon().meters,
        &SubjectRoll {
            rule_id: check_id,
            actor_kind: kind,
            actor_id,
            check_id,
            target,
            partial_at,
            live: Some(&actor),
        },
    )
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
