//! Exits, admission, and the journeys that follow them.

use crate::canon::{Canon, Exit, PlaceDef};
use crate::ecs::id::ActorId;
use crate::ecs::{self, component, PendingAction, Presence};
use crate::intent::{Intent, Source, Target, Viewer};
use crate::round::log::RoundLog;
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::effect::{Effect, EffectKind};
use crate::rules::gate;
use crate::rules::lines::find_place_name;
use crate::rules::rule::{EffectTemplate, LocationTo, Rule};
use crate::rules::visibility::can_see_fact;
use crate::Reason;
use spookery_protocol::ExitView;

pub(crate) fn find_exit<'a>(canon: &'a Canon, from: &str, exit_id: &str) -> Option<&'a Exit> {
    canon
        .places
        .get(from)
        .and_then(|place| place.exits.iter().find(|exit| exit.id == exit_id))
}

/// The exit this rule travels by: the one its first `location via` that is not a teleport
/// names. `None` when the rule does not travel.
pub(crate) fn exit_of<'a>(
    context: &Context<'a, '_>,
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
/// exit target, and `None` when the target is not an exit; any other word is an exit id.
/// A walker on an exit is still on it when the walk finishes, so it is that exit.
pub(crate) fn exit_via<'a>(
    context: &Context<'a, '_>,
    via: &str,
) -> Result<Option<&'a Exit>, Reason> {
    let exit_id = match (via, &context.intent.target) {
        ("target", Target::Exit(exit_id)) => exit_id.as_str(),
        ("target", _) => return Ok(None),
        (exit_id, _) => exit_id,
    };
    let from = context
        .intent
        .actor_id()
        .and_then(|id| context.world.actors().get::<component::Place>(id).get())
        .unwrap_or("");
    if let Some(exit) = exit_stood_on(context.canon, from) {
        return Ok(Some(exit));
    }
    find_exit(context.canon, from, exit_id)
        .map(Some)
        .ok_or(Reason::NoExit)
}

/// The exit a walker stands on, from the place id `exit:<place>.<exit>` they hold while
/// walking. `None` for a place.
pub(crate) fn exit_stood_on<'a>(canon: &'a Canon, place: &str) -> Option<&'a Exit> {
    let (from, exit_id) = crate::ecs::exit_parts(place)?;
    find_exit(canon, from, exit_id)
}

/// Where a walker stands while they walk through `exit`: `exit:<place>.<exit>`.
pub(crate) fn on_exit(exit: &Exit) -> String {
    exit.qualified_id().to_string()
}

/// Refuse a missing, hidden, locked, or unpaid exit. A clear exit is `Ok`.
pub(crate) fn admit(context: &Context, rule: &Rule) -> Result<(), gate::Refusal> {
    let Some(exit) = exit_of(context, rule)? else {
        return Ok(());
    };
    let (canon, world, intent) = (context.canon, context.world, context.intent);
    let actor_id = intent
        .actor_id()
        .ok_or_else(|| gate::Refusal::new(intent, Reason::UnknownId))?;
    if !is_revealed(canon, world, actor_id, exit) {
        return Err(gate::Refusal::new(intent, Reason::NoExit));
    }
    if is_locked(world, exit) {
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

fn is_revealed(canon: &Canon, world: &ecs::World, actor_id: &ActorId, exit: &Exit) -> bool {
    if !exit.hidden {
        return true;
    }
    let Some(fact) = &exit.revealed_by else {
        return false;
    };
    let Some(viewer) = (world.actors().get::<component::ControlType>(actor_id)).viewer() else {
        return false;
    };
    can_see_fact(canon, world, &viewer, fact)
}

/// A door is locked until it opens. An exit with no `if` is no door, so it is always open.
fn is_locked(world: &ecs::World, exit: &Exit) -> bool {
    !exit.if_.is_empty() && !world.places().is_exit_open(&exit.qualified_id())
}

/// Open each locked door whose `if` some actor in its place meets, with an `exit_open` effect
/// written as no one, under the intent `door:<place>.<exit>`. Runs at the end of each round,
/// after triggers.
pub(crate) fn open_doors(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    log: &mut RoundLog,
) {
    let mut opens = Vec::new();
    for (place, def) in &canon.places {
        let here: Vec<Viewer> = world
            .actors_at(place.as_str())
            .iter()
            .filter_map(|id| world.actors().get::<component::ControlType>(id).viewer())
            .collect();
        for exit in def.exits.iter().filter(|exit| is_locked(world, exit)) {
            let target = Target::Exit(exit.id.clone());
            let met = here.iter().any(|viewer| {
                let target = target.clone();
                cond::if_holds(canon, world, presence, viewer, "travel", target, &exit.if_)
            });
            if !met {
                continue;
            }
            let id = format!("door:{place}.{}", exit.id);
            let intent = Intent::new(id, world.round_index, None, Source::Engine, "door", target);
            let kind = EffectKind::ExitOpen {
                exit: exit.qualified_id(),
                open: true,
            };
            opens.push((Effect::new(None, &intent.id, kind), intent));
        }
    }
    for (effect, intent) in opens {
        let point = world.savepoint();
        match effect.clone().apply(canon, world, presence, &intent) {
            Ok(()) => log.effects.push(effect),
            Err(code) => {
                world.rollback(point);
                log.refused.push(gate::Refusal::new(&intent, code));
            }
        }
    }
}

/// The exits of `place` this actor is allowed to see. A hidden exit stays off the list.
pub(crate) fn exits_at(
    canon: &Canon,
    world: &ecs::World,
    actor_id: &ActorId,
    place: &PlaceDef,
) -> Vec<ExitView> {
    place
        .exits
        .iter()
        .filter(|exit| is_revealed(canon, world, actor_id, exit))
        .map(|exit| exit_view(canon, world, exit))
        .collect()
}

/// `exit` as it is now: where it goes, its ticks, and whether it is locked.
pub(crate) fn exit_view(canon: &Canon, world: &ecs::World, exit: &Exit) -> ExitView {
    ExitView {
        exit_id: exit.id.clone(),
        name: exit.name.clone(),
        place_id: exit.to.clone(),
        // Load refuses an exit whose `to` is not a place, so the place has a name.
        place_name: find_place_name(canon, &exit.to).unwrap_or_default(),
        ticks: exit.cost.ticks,
        locked: is_locked(world, exit),
    }
}

/// Every actor notes the exits they see where they stand, for `map`. Runs once at the end
/// of each round, after triggers, so a reveal is seen in the round it lands.
pub(crate) fn note_exits(canon: &Canon, world: &mut ecs::World) -> Result<(), Reason> {
    // A walker on an exit is in no place, so there is nothing to note.
    let actors = world.actors().query::<(ActorId, &component::Place)>();
    let seen: Vec<(ActorId, String, Vec<String>)> = (actors.iter())
        .filter(|(_, place)| canon.places.contains_key(place.as_str()))
        .map(|(id, place)| {
            let exits = visible_exits(canon, world, id);
            let exit_ids = exits.into_iter().map(|exit| exit.exit_id).collect();
            (id.clone(), place.clone(), exit_ids)
        })
        .collect();
    let mut actors = world.actors_mut();
    for (actor, place, exits) in seen {
        (actors.get_mut::<component::Visit>(&actor)).set(&place, exits)?;
    }
    Ok(())
}

/// The exits of this actor's place that they are allowed to see.
pub(crate) fn visible_exits(
    canon: &Canon,
    world: &ecs::World,
    actor_id: &ActorId,
) -> Vec<ExitView> {
    let place = world.actors().get::<component::Place>(actor_id).get();
    match place.and_then(|place| canon.places.get(place)) {
        Some(place) => exits_at(canon, world, actor_id, place),
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
    pub actor_id: ActorId,
    pub until: u64,
    pub pending: PendingAction,
}

/// Clear every action that finishes at or before `end`, earliest first.
pub(crate) fn take_due_actions(world: &mut ecs::World, end: u64) -> Result<Vec<DueAction>, Reason> {
    let mut due_keys: Vec<(u64, ActorId)> = (world.actors().query::<(ActorId, &component::Act)>())
        .iter()
        .filter_map(|(id, act)| {
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
        let mut actors = world.actors_mut();
        let Some(pending) = actors.get_mut::<component::Act>(&actor_id).take_pending()? else {
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
