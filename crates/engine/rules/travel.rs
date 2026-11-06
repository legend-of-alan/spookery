//! Exits, admission, and the journeys that follow them.

use crate::canon::{Canon, Exit, PlaceDef, OPEN};
use crate::draft::Draft;
use crate::intent::{Intent, Source, Target, Viewer};
use crate::model::tracked::Entry;
use crate::model::{ActorEdit, PendingAction, State};
use crate::round::log::RoundLog;
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::effect::{Effect, EffectKind, Holder};
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
/// A walker on an exit is still on it when the walk finishes, so it is that exit.
pub(crate) fn exit_via<'a>(context: &Context<'a>, via: &str) -> Result<Option<&'a Exit>, Reason> {
    let name = match (via, &context.intent.target) {
        ("target", Target::Exit(name)) => name.as_str(),
        ("target", _) => return Ok(None),
        (name, _) => name,
    };
    let actor = context
        .intent
        .actor_id()
        .and_then(|id| context.state.actors.get(id));
    let from = actor.map_or("", |actor| actor.place.as_str());
    if let Some(exit) = exit_stood_on(context.canon, from) {
        return Ok(Some(exit));
    }
    find_exit(context.canon, from, name)
        .map(Some)
        .ok_or(Reason::NoExit)
}

/// The exit a walker stands on, from the place id `exit:<place>.<exit>` they hold while
/// walking. `None` for a place.
pub(crate) fn exit_stood_on<'a>(canon: &'a Canon, place: &str) -> Option<&'a Exit> {
    let (from, name) = crate::model::exit_parts(place)?;
    find_exit(canon, from, name)
}

/// Where a walker stands while they walk through `exit`: `exit:<place>.<exit>`.
pub(crate) fn on_exit(exit: &Exit) -> String {
    door(exit).to_string()
}

/// Refuse a missing, hidden, locked, or unpaid exit. A clear exit is `Ok`.
pub(crate) fn admit(context: &Context, rule: &Rule) -> Result<(), gate::Refusal> {
    let Some(exit) = exit_of(context, rule)? else {
        return Ok(());
    };
    let (canon, state, intent) = (context.canon, context.state, context.intent);
    let actor_id = intent
        .actor_id()
        .ok_or_else(|| gate::Refusal::new(intent, Reason::UnknownId))?;
    if !is_revealed(canon, state, actor_id, exit) {
        return Err(gate::Refusal::new(intent, Reason::NoExit));
    }
    if is_locked(state, exit) {
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

fn is_revealed(canon: &Canon, state: &State, actor_id: &str, exit: &Exit) -> bool {
    if !exit.hidden {
        return true;
    }
    let Some(fact) = &exit.revealed_by else {
        return false;
    };
    let Some(viewer) = state.viewer_of(actor_id) else {
        return false;
    };
    can_see_fact(canon, state, &viewer, fact)
}

/// A door is locked until it opens. An exit with no `if` is no door, so it is always open.
fn is_locked(state: &State, exit: &Exit) -> bool {
    !exit.if_.is_empty() && !state.has_flag(&door(exit), OPEN)
}

/// The exit as the holder of its flag `open`.
fn door(exit: &Exit) -> Holder {
    Holder::Exit(exit.from.clone(), exit.name.clone())
}

/// Open each locked door whose `if` some actor in its place meets, with a `flag` effect
/// written as no one, under the intent `door:<place>.<exit>`. Runs at the end of each round,
/// after triggers.
pub(crate) fn open_doors(draft: &mut Draft, log: &mut RoundLog) {
    let (canon, state) = (draft.canon(), draft.state());
    let mut opens = Vec::new();
    for (place, def) in &canon.places {
        let here: Vec<Viewer> = state
            .actors_at(place)
            .iter()
            .filter_map(|id| state.viewer_of(id))
            .collect();
        for exit in def.exits.iter().filter(|exit| is_locked(state, exit)) {
            let target = Target::Exit(exit.name.clone());
            let met = here.iter().any(|viewer| {
                let target = target.clone();
                cond::if_holds(
                    canon,
                    state,
                    draft.presence,
                    viewer,
                    "travel",
                    target,
                    &exit.if_,
                )
            });
            if !met {
                continue;
            }
            let id = format!("door:{place}.{}", exit.name);
            let intent = Intent::new(id, state.round_index, None, Source::Engine, "door", target);
            let kind = EffectKind::Flag {
                holder: door(exit),
                key: OPEN.to_string(),
                value: true,
            };
            opens.push((Effect::new(None, &intent.id, kind), intent));
        }
    }
    for (effect, intent) in opens {
        let point = draft.savepoint();
        match effect.clone().apply(draft, &intent) {
            Ok(()) => log.effects.push(effect),
            Err(code) => {
                draft.rollback(point);
                log.refused.push(gate::Refusal::new(&intent, code));
            }
        }
    }
}

/// The exits of `place` this actor is allowed to see. A hidden exit stays off the list.
pub(crate) fn exits_at(
    canon: &Canon,
    state: &State,
    actor_id: &str,
    place: &PlaceDef,
) -> Vec<ExitView> {
    place
        .exits
        .iter()
        .filter(|exit| is_revealed(canon, state, actor_id, exit))
        .map(|exit| exit_view(canon, state, exit))
        .collect()
}

/// `exit` as it is now: where it goes, its ticks, and whether it is locked.
pub(crate) fn exit_view(canon: &Canon, state: &State, exit: &Exit) -> ExitView {
    ExitView {
        name: exit.name.clone(),
        to: exit.to.clone(),
        place_name: place_name(canon, &exit.to),
        ticks: exit.cost.ticks,
        locked: is_locked(state, exit),
    }
}

/// Every actor notes the exits they see where they stand, for `map`. Runs once at the end
/// of each round, after triggers, so a reveal is seen in the round it lands.
pub(crate) fn note_exits(draft: &mut Draft) -> Result<(), Reason> {
    let canon = draft.canon();
    let state = draft.state();
    // A walker on an exit is in no place, so there is nothing to note.
    let seen: Vec<(String, ActorEdit)> = state
        .actors
        .iter()
        .filter(|(_, actor)| canon.places.contains_key(&actor.place))
        .map(|(id, actor)| {
            let exits = visible_exits(canon, state, id);
            let exits = Some(exits.into_iter().map(|exit| exit.name).collect());
            let place = actor.place.clone();
            let edit = ActorEdit::Visited(Entry {
                id: place,
                value: exits,
            });
            (id.clone(), edit)
        })
        .collect();
    seen.into_iter()
        .try_for_each(|(id, edit)| draft.edit_actor(&id, edit))
}

/// The exits of this actor's place that they are allowed to see.
pub(crate) fn visible_exits(canon: &Canon, state: &State, actor_id: &str) -> Vec<ExitView> {
    let actor = state.actors.get(actor_id);
    match actor.and_then(|actor| canon.places.get(&actor.place)) {
        Some(place) => exits_at(canon, state, actor_id, place),
        None => Vec::new(),
    }
}

/// Ticks until the rule and its exit finish. Zero means it finishes now.
pub(crate) fn span(context: &Context, rule: &Rule) -> u64 {
    let mut ticks = rule.start.cost.as_ref().map_or(0, |cost| cost.ticks);
    if let Ok(Some(exit)) = exit_of(context, rule) {
        ticks = ticks.saturating_add(exit.cost.ticks);
    }
    ticks
}

/// One action whose finish tick has been reached. The actor is no longer busy.
pub(crate) struct DueAction {
    pub actor_id: String,
    pub until: u64,
    pub pending: PendingAction,
}

/// Clear every action that finishes at or before `end`, earliest first.
pub(crate) fn take_due_actions(draft: &mut Draft, end: u64) -> Result<Vec<DueAction>, Reason> {
    let mut due_keys: Vec<(u64, String)> = draft
        .state()
        .actors
        .iter()
        .filter_map(|(id, actor)| {
            let act = &actor.act;
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
        let Some(pending) = draft.take_pending(&actor_id)? else {
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
