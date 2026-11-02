//! Exits, admission, and the journeys that follow them.

use crate::canon::{Canon, Exit, PlaceDef};
use crate::draft::Draft;
use crate::intent::{Intent, Target};
use crate::model::key::StateKey;
use crate::model::{ActorLive, PendingAction, PendingKind, Presence, State};
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::expr::Dice;
use crate::rules::gate;
use crate::rules::lines::place_name;
use crate::rules::rule::{EffectTemplate, LocationTo, Rule};
use crate::rules::visibility::can_see_fact;
use crate::Reason;
use spookery_protocol::ExitView;

pub(crate) fn find_exit<'a>(canon: &'a Canon, from: &str, name: &str) -> Option<&'a Exit> {
    canon
        .places
        .get(from)
        .and_then(|place| place.exits.iter().find(|exit| exit.name == name))
}

/// The exit this rule travels by: the one its first `location via` that is not a teleport
/// names. `None` when the rule does not travel.
pub(crate) fn exit_of<'a>(
    context: &Context<'a>,
    rule: &Rule,
) -> Result<Option<&'a Exit>, gate::Refusal> {
    let via = rule.top_templates().find_map(|effect| match effect {
        EffectTemplate::Location {
            to: LocationTo::Via(via),
            teleport: false,
        } => Some(via),
        _ => None,
    });
    match via {
        Some(via) => {
            exit_via(context, via).map_err(|code| gate::Refusal::new(context.intent, code))
        }
        None => Ok(None),
    }
}

/// The exit `via` names from the actor's place, or `NO_EXIT`. `via: target` is the intent's
/// exit target, and `None` when the target is not an exit; any other word is an exit name.
pub(crate) fn exit_via<'a>(context: &Context<'a>, via: &str) -> Result<Option<&'a Exit>, Reason> {
    let name = match (via, &context.intent.target) {
        ("target", Target::Exit(name)) => name.as_str(),
        ("target", _) => return Ok(None),
        (name, _) => name,
    };
    let actor = context.state.actors.get(context.intent.actor.actor_id());
    let from = actor.map(ActorLive::place).unwrap_or("");
    find_exit(context.canon, from, name)
        .map(Some)
        .ok_or(Reason::NoExit)
}

/// Refuse a missing, hidden, locked, or unpaid exit. A clear exit is `Ok`.
pub(crate) fn admit(context: &Context, rule: &Rule) -> Result<(), gate::Refusal> {
    let Some(exit) = exit_of(context, rule)? else {
        return Ok(());
    };
    let (canon, now, intent) = (context.canon, context.state, context.intent);
    if !is_revealed(canon, now, intent.actor.actor_id(), exit) {
        return Err(gate::Refusal::new(intent, Reason::NoExit));
    }
    if is_locked(canon, now, context.presence, intent.actor.actor_id(), exit) {
        let text = if exit.locked_text.is_empty() {
            "The way is locked.".to_string()
        } else {
            exit.locked_text.clone()
        };
        return Err(gate::Refusal {
            id: intent.id.clone(),
            code: Reason::NoExit,
            detail: Some(text),
        });
    }
    cost::afford(context, &exit.cost)
}

fn is_revealed(canon: &Canon, now: &State, actor_id: &str, exit: &Exit) -> bool {
    if !exit.hidden {
        return true;
    }
    let Some(fact) = &exit.revealed_by else {
        return false;
    };
    let Some(viewer) = now.viewer_of(actor_id) else {
        return false;
    };
    can_see_fact(canon, now, &viewer, fact)
}

fn is_locked(canon: &Canon, now: &State, presence: &Presence, actor_id: &str, exit: &Exit) -> bool {
    if exit.if_.is_empty() {
        return false;
    }
    let Some(viewer) = now.viewer_of(actor_id) else {
        return true;
    };
    let target = Target::Exit(exit.name.clone());
    !cond::if_holds(canon, now, presence, &viewer, "travel", target, &exit.if_)
}

/// The exits of `place` this actor is allowed to see. A hidden exit stays off the list.
pub(crate) fn exits_at(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
    place: &PlaceDef,
) -> Vec<ExitView> {
    place
        .exits
        .iter()
        .filter(|exit| is_revealed(canon, now, actor_id, exit))
        .map(|exit| ExitView {
            name: exit.name.clone(),
            to: exit.to.clone(),
            place_name: place_name(canon, &exit.to),
            minutes: exit.cost.minutes,
            locked: is_locked(canon, now, presence, actor_id, exit),
        })
        .collect()
}

/// The exits of this actor's place that they are allowed to see.
pub(crate) fn visible_exits(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
) -> Vec<ExitView> {
    let actor = now.actors.get(actor_id);
    match actor.and_then(|actor| canon.places.get(actor.place())) {
        Some(place) => exits_at(canon, now, presence, actor_id, place),
        None => Vec::new(),
    }
}

/// World minutes until the rule and its exit finish. Zero means it finishes now.
pub(crate) fn span(context: &Context, rule: &Rule) -> u64 {
    let mut minutes = rule.cost.as_ref().map(|cost| cost.minutes).unwrap_or(0);
    if let Ok(Some(exit)) = exit_of(context, rule) {
        minutes = minutes.saturating_add(exit.cost.minutes);
    }
    minutes
}

/// One journey whose minute has been reached. The actor is no longer busy.
pub(crate) struct DueAction {
    pub actor_id: String,
    pub until: u64,
    pub pending: PendingAction,
}

/// Clear every journey that finishes at or before `end`, earliest first.
pub(crate) fn take_due_actions(draft: &mut Draft, end: u64) -> Result<Vec<DueAction>, Reason> {
    let mut due_keys: Vec<(u64, String)> = draft
        .state()
        .actors
        .iter()
        .filter_map(|(id, actor)| {
            let act = actor.act();
            let until = act.busy_until?;
            if act.pending.is_none() || until > end {
                return None;
            }
            Some((until, id.clone()))
        })
        .collect();
    due_keys.sort();
    let mut due_actions = Vec::new();
    for (until, actor_id) in due_keys {
        draft.track(StateKey::ActorBusy(actor_id.clone()))?;
        let pending = {
            let actor = draft
                .state_mut()
                .actors
                .get_mut(&actor_id)
                .ok_or(Reason::UnknownId)?;
            let act = actor.act_mut();
            act.busy_until = None;
            act.pending.take()
        };
        let Some(pending) = pending else {
            continue;
        };
        due_actions.push(DueAction {
            actor_id,
            until,
            pending,
        });
    }
    Ok(due_actions)
}

/// Pay the exit, then remember the finish minute. The move itself waits.
pub(crate) fn begin_action(
    draft: &mut Draft,
    intent: &Intent,
    rule: &Rule,
    world_time_before: u64,
    minutes: u64,
    dice: &mut dyn Dice,
) -> Result<(), Reason> {
    let context = Context::new(draft.canon(), draft.state(), draft.presence, intent);
    let kind = match exit_of(&context, rule).ok().flatten().cloned() {
        Some(exit) => {
            cost::pay(draft, intent, &exit.cost, dice)?;
            PendingKind::Arrive {
                place: exit.to,
                via: exit.name,
            }
        }
        None => PendingKind::Finish {
            target: intent.target.clone(),
            rule: rule.id.clone(),
        },
    };
    let until = world_time_before.saturating_add(minutes);
    let pending = PendingAction {
        verb: intent.move_name.clone(),
        source: intent.source,
        kind,
    };
    draft.track(StateKey::ActorBusy(intent.actor.actor_id().to_string()))?;
    let act = draft
        .state_mut()
        .actors
        .get_mut(intent.actor.actor_id())
        .ok_or(Reason::UnknownId)?
        .act_mut();
    act.busy_until = Some(until);
    act.pending = Some(pending);
    Ok(())
}
