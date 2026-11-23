//! Exits, admission, and the journeys that follow them.

use crate::canon::{Canon, Exit};
use crate::ecs::id::{ActorId, GlobalId};
use crate::ecs::{self, component, PendingAction, Presence};
use crate::intent::{Intent, Source, Target};
use crate::round::Ledger;
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::effect::{Effect, EffectKind};
use crate::rules::gate;
use crate::rules::lines::find_place_name;
use crate::rules::rule::{EffectTemplate, LocationTo, Rule};
use crate::Reason;
use spookery_protocol::ExitView;

/// The exit this rule travels by: the one its first `location via` that is not a teleport
/// names, in a nested check too. `None` when the rule does not travel.
pub(crate) fn exit_of<'c>(
    context: &Context<'c, '_>,
    rule: &Rule,
) -> Result<Option<Exit<'c>>, gate::Refusal> {
    let via = (rule.templates().into_iter()).find_map(|effect| match effect {
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
pub(crate) fn exit_via<'c>(
    context: &Context<'c, '_>,
    via: &str,
) -> Result<Option<Exit<'c>>, Reason> {
    let exit_id = match (via, &context.intent.target) {
        ("target", Target::Exit(exit_id)) => exit_id.as_str(),
        ("target", _) => return Ok(None),
        (exit_id, _) => exit_id,
    };
    let from = context
        .intent
        .actor_id
        .as_ref()
        .and_then(|id| context.world.actors().get::<component::Place>(id).get())
        .unwrap_or("");
    if let Some(exit) = exit_stood_on(context.canon, from) {
        return Ok(Some(exit));
    }
    (context.canon.find_exit(from, exit_id))
        .map(Some)
        .ok_or(Reason::NoExit)
}

/// The exit a walker stands on, from the place id `exit:<place>.<exit>` they hold while
/// walking. `None` for a place.
pub(crate) fn exit_stood_on<'c>(canon: &'c Canon, place: &str) -> Option<Exit<'c>> {
    let (from, exit_id) = crate::ecs::exit_parts(place)?;
    canon.find_exit(from, exit_id)
}

/// Refuse a missing, hidden, locked, or unpaid exit. A clear exit is `Ok`.
pub(crate) fn admit(context: &Context, rule: &Rule) -> Result<(), gate::Refusal> {
    let Some(exit) = exit_of(context, rule)? else {
        return Ok(());
    };
    let (world, intent) = (context.world, context.intent);
    let actor_id = intent
        .actor_id
        .as_ref()
        .ok_or_else(|| gate::Refusal::new(intent, Reason::UnknownId))?;
    if !is_revealed(world, actor_id, exit) {
        return Err(gate::Refusal::new(intent, Reason::NoExit));
    }
    if is_locked(world, exit) {
        let text = if exit.path.locked_text.is_empty() {
            "The way is locked.".to_string()
        } else {
            exit.path.locked_text.clone()
        };
        return Err(gate::Refusal {
            id: intent.id.clone(),
            code: Reason::NoExit,
            detail: Some(text),
        });
    }
    cost::afford(context, &exit.path.cost)
}

fn is_revealed(world: &ecs::World, actor_id: &ActorId, exit: Exit<'_>) -> bool {
    let Some(key) = &exit.path.revealed_by else {
        return true;
    };
    world
        .actors()
        .get::<component::Flag>(actor_id)
        .contains(key)
        || world
            .globals()
            .get::<component::Flag>(&GlobalId)
            .contains(key)
}

/// An exit with an `if` is locked until it opens. An exit with no `if` is always open.
fn is_locked(world: &ecs::World, exit: Exit<'_>) -> bool {
    exit.path.is_conditional() && !world.places().is_exit_open(&exit.exit_id())
}

/// Open the conditional exits whose `if` holds, with an `exit_open` effect
/// written as no one, under the intent `open:<place>.<exit>`. Runs at the end of each round,
/// after triggers.
pub(crate) fn open_conditional_exits(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    ledger: &mut Ledger,
) {
    let mut opens = Vec::new();
    for place in canon.places.keys() {
        let here = world.actors_at(place.as_str());
        for exit in canon
            .exits(place.as_str())
            .filter(|exit| is_locked(world, *exit))
        {
            let target = Target::Exit(exit.id.to_string());
            let met = here.iter().any(|actor_id| {
                let target = target.clone();
                cond::if_holds(
                    canon,
                    world,
                    presence,
                    actor_id,
                    "travel",
                    target,
                    &exit.path.if_,
                )
            });
            if !met {
                continue;
            }
            let id = format!("open:{place}.{}", exit.id);
            let intent = Intent::new(id, world.round_index, None, Source::Engine, "open", target);
            let kind = EffectKind::ExitOpen {
                exit: exit.exit_id(),
                open: true,
            };
            opens.push((Effect::new(None, &intent.id, kind), intent));
        }
    }
    for (effect, intent) in opens {
        let point = world.savepoint();
        match effect.clone().apply(canon, world, presence, &intent) {
            Ok(()) => ledger.effects.push(effect),
            Err(code) => {
                world.rollback(point);
                ledger.refused.push(gate::Refusal::new(&intent, code));
            }
        }
    }
}

/// The exits of `place` this actor is allowed to see. An exit not yet revealed to them stays
/// off the list.
pub(crate) fn exits_at(
    canon: &Canon,
    world: &ecs::World,
    actor_id: &ActorId,
    place: &str,
) -> Vec<ExitView> {
    (canon.exits(place))
        .filter(|exit| is_revealed(world, actor_id, *exit))
        .map(|exit| exit_view(canon, world, actor_id, exit))
        .collect()
}

/// `exit` as `actor_id` sees it now: where it goes, its ticks read for their walk (0 when they
/// cannot be read), and whether it is locked. The ticks are read against a travel intent with no
/// id, with no one seated, since a number reads no presence.
pub(crate) fn exit_view(
    canon: &Canon,
    world: &ecs::World,
    actor_id: &ActorId,
    exit: Exit<'_>,
) -> ExitView {
    let travel_intent = Intent::new(
        String::new(),
        world.round_index,
        Some(actor_id.clone()),
        Source::Player,
        "travel",
        Target::Exit(exit.id.to_string()),
    );
    let presence = Presence::default();
    let context = Context::new(canon, world, &presence, &travel_intent);
    ExitView {
        exit_id: exit.id.to_string(),
        name: exit.name(canon).to_string(),
        place_id: exit.to.to_string(),
        place_name: find_place_name(canon, exit.to.as_str()).unwrap_or_default(),
        ticks: cost::read_ticks(&context, &exit.path.cost).unwrap_or(0),
        locked: is_locked(world, exit),
    }
}

/// Every actor notes the exits they see where they stand, for `map`. Runs once at the end
/// of each round, after triggers, so an exit a flag reveals is seen in the round the flag is set.
pub(crate) fn note_exits(canon: &Canon, world: &mut ecs::World) -> Result<(), Reason> {
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
    place.map_or(Vec::new(), |place| exits_at(canon, world, actor_id, place))
}

/// Ticks until the rule and its exit finish, read for the acting actor. Zero means it
/// finishes now. A negative count is `OUT_OF_RANGE`.
pub(crate) fn span(context: &Context, rule: &Rule) -> Result<u64, Reason> {
    let start = rule.start.cost.as_ref();
    let mut ticks = start.map_or(Ok(0), |cost| cost::read_ticks(context, cost))?;
    if let Ok(Some(exit)) = exit_of(context, rule) {
        ticks = ticks.saturating_add(cost::read_ticks(context, &exit.path.cost)?);
    }
    Ok(ticks)
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
