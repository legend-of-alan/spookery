//! Finish the timed actions whose minute this round reached.

use crate::apply::apply_effect;
use crate::draft::Draft;
use crate::effect::{Effect, EffectKind};
use crate::expand::expand_rule;
use crate::intent::{ActorKind, Intent, IntentArgs, Target, ViewerId};
use crate::journal::Arrival;
use crate::model::PendingAction;
use crate::rules::MoveReport;
use crate::Reason;

use crate::round::dice::{ResistKey, RoundDice};
use crate::round::log::RoundLog;

/// Finish every due action, earliest first.
pub(crate) fn run(
    draft: &mut Draft,
    end: u64,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<(), Reason> {
    let due_actions = crate::map::take_due_actions(draft, end)?;
    for due_action in due_actions {
        if due_action.pending.arrive.is_some() {
            arrive(
                log,
                draft,
                &due_action.actor_id,
                due_action.until,
                &due_action.pending,
            )?;
        } else {
            perform(dice, log, draft, &due_action.actor_id, &due_action.pending)?;
        }
    }
    Ok(())
}

fn arrive(
    log: &mut RoundLog,
    draft: &mut Draft,
    actor_id: &str,
    until: u64,
    pending: &PendingAction,
) -> Result<(), Reason> {
    let actor = draft.now().actors.get(actor_id).ok_or(Reason::UnknownKey)?;
    let from = actor.place().to_string();
    let to = pending.arrive.clone().unwrap_or_default();
    let kind = actor.actor_kind();
    let intent = due_intent(draft.now().round_index, actor_id, kind, pending);
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
    log.effects.push(effect);
    log.arrivals.push(Arrival {
        actor_id: actor_id.to_string(),
        arrived_at: until,
        to,
        via: pending.via.clone().unwrap_or_default(),
    });
    Ok(())
}

fn perform(
    dice: &mut RoundDice,
    log: &mut RoundLog,
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
    let intent = due_intent(draft.now().round_index, actor_id, kind, pending);
    let canon = draft.canon();
    let snapshot = draft.now();
    let rule = crate::rules::gate::resolve(canon, snapshot, &intent)?;
    let mut roller = dice.roller(log, &intent.move_name, actor_id, ResistKey::Check);
    let rolled = match &rule.roll {
        Some(spec) => Some(roller.roll_spec(canon, snapshot, &intent, spec, &spec.check)?),
        None => None,
    };
    let outcome = rolled.as_ref().map(|out| out.outcome);
    let effects = expand_rule(canon, snapshot, rule, &intent, outcome, &mut roller)?;
    let resisted = roller.finish();
    let actor_id = actor_id.to_string();
    log.reports
        .extend(rolled.map(|out| MoveReport::Rolled { actor_id, out }));
    log.reports.extend(resisted);
    for effect in effects {
        if matches!(effect.class, EffectKind::Location { .. }) {
            continue;
        }
        apply_effect(draft, &intent, effect.clone())?;
        log.effects.push(effect);
    }
    Ok(())
}

fn due_intent(
    round_index: u64,
    actor_id: &str,
    kind: ActorKind,
    pending: &PendingAction,
) -> Intent {
    Intent {
        id: format!("arrival:{actor_id}"),
        round_index,
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
