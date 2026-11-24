//! Who a rule line names: the item `id: target` stands for, the owner an `of:` names, the actor
//! an actor word names, and the place `here` is.

use crate::ecs::component;
use crate::ecs::id::{ActorId, PlaceId};
use crate::entity;
use crate::intent::Target;
use crate::rules::context::Context;
use crate::rules::grammar;
use crate::Reason;

/// `id: target` is the intent's target token. Any other id is itself.
pub(crate) fn resolve_item_id(context: &Context, id: &str) -> Result<String, Reason> {
    if id == "target" {
        return context
            .intent
            .target
            .token()
            .map(str::to_string)
            .ok_or(Reason::Schema);
    }
    Ok(id.to_string())
}

/// Who holds the meter, flag or item a rule names: `here` and `place:` are a place, `path:`
/// is a path, `story:` is a story, `world` is the world, `target` on a rule that targets an
/// exit is that exit's path, and anyone else is an actor.
pub(crate) fn resolve_entity(
    context: &Context,
    of: &grammar::Object,
) -> Result<entity::Id, Reason> {
    match of {
        grammar::Object::Global => Ok(entity::Id::Global),
        grammar::Object::Story(id) => Ok(entity::Id::Story(id.clone())),
        grammar::Object::Path(id) => Ok(entity::Id::Path(id.clone())),
        grammar::Object::Place(id) => Ok(entity::Id::Place(id.clone())),
        grammar::Object::Here => Ok(entity::Id::Place(resolve_here(context)?.into())),
        grammar::Object::Actor(grammar::Actor::Target)
            if matches!(context.intent.target, Target::Exit(_)) =>
        {
            let exit = crate::rules::travel::exit_via(context, "target")?;
            let path = exit.ok_or(Reason::NoExit)?.path;
            Ok(entity::Id::Path(path.id.clone()))
        }
        grammar::Object::Actor(actor) => {
            Ok(entity::Id::Actor(resolve_actor(context, actor)?.clone()))
        }
    }
}

/// The actor `actor` names: the one acting, the move's target, or an actor by id. A target
/// that is a place, an exit or an item names no actor, so it is `UNKNOWN_ID`.
pub(crate) fn resolve_actor<'a>(
    context: &Context<'_, 'a>,
    actor: &'a grammar::Actor,
) -> Result<&'a ActorId, Reason> {
    match actor {
        grammar::Actor::Self_ => context.intent.actor_id.as_ref().ok_or(Reason::Schema),
        grammar::Actor::Target => match &context.intent.target {
            Target::Actor(id) => Ok(id),
            Target::Empty => Err(Reason::Schema),
            Target::Place(_) | Target::Exit(_) | Target::Item(_) => Err(Reason::UnknownId),
        },
        grammar::Actor::Id(id) => Ok(id),
    }
}

/// The place the acting actor stands in. A walker is in no place, so it is `UNKNOWN_ID`.
pub(crate) fn resolve_here<'a>(context: &Context<'_, 'a>) -> Result<&'a str, Reason> {
    let actor_id = context.intent.actor_id.as_ref().ok_or(Reason::Schema)?;
    let place = context.world.actors().get::<component::Place>(actor_id);
    place
        .place_id()
        .map(PlaceId::as_str)
        .ok_or(Reason::UnknownId)
}
