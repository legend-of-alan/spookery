//! The place name, place prose and busy line a view carries, and a player's help and map.

use spookery_protocol::{CommandView, PlaceView, Seat, COMMANDS};

use crate::canon::Canon;
use crate::ecs::id::ActorId;
use crate::ecs::{self, component, Presence};
use crate::intent::Target;
use crate::rules::cond;
use crate::rules::gate::{list_ids, verb_for, NamedId};
use crate::rules::rule::{Owner, TargetKind};
use crate::rules::travel::{exit_view, exits_at};
use crate::Reason;

/// `help`: the built-in player commands but `claim`, then each pack verb `verb_for` reaches
/// now and whose `if` holds. A cast member's or item's verb is reached by naming them
/// (`list_ids`), so it carries that owner as its one name. Each name is the one the
/// player reads. Cost is not checked.
pub(crate) fn help(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    actor_id: &ActorId,
) -> Result<Vec<CommandView>, Reason> {
    if !world.actors().contains(actor_id) {
        return Err(Reason::UnknownId);
    }
    let mut commands: Vec<CommandView> = COMMANDS
        .iter()
        .filter(|command| command.seat == Seat::Player && command.id != "claim")
        .map(|command| CommandView {
            id: command.id.to_string(),
            target: TargetKind::None.as_str().to_string(),
            names: Vec::new(),
            owner: None,
        })
        .collect();
    let ids = list_ids(canon, world, presence, actor_id);
    let name_of = |id: &String| match ids.iter().find(|named_id| named_id.id == *id) {
        Some(named_id) => Ok(named_id.name.clone()),
        None => find_place_name(canon, id).ok_or(Reason::UnknownId),
    };
    let hints: Vec<Option<&str>> = std::iter::once(None)
        .chain(ids.iter().map(|named_id| Some(named_id.id.as_str())))
        .collect();
    for verb_id in canon.verbs() {
        for &hint in &hints {
            let Some(rule) = verb_for(canon, world, actor_id, &verb_id, hint) else {
                continue;
            };
            let (owner, target) = match (&rule.owner, hint) {
                (_, None) => (None, Target::Empty),
                (Owner::Cast(id), Some(hint)) if id == hint => {
                    (Some(id), Target::Actor(id.clone().into()))
                }
                (Owner::Item(id), Some(hint)) if id == hint => (Some(id), Target::Item(id.clone())),
                _ => continue,
            };
            if !cond::if_holds(
                canon, world, presence, actor_id, &verb_id, target, &rule.if_,
            ) {
                continue;
            }
            let kind = rule.when.target_kind();
            let target_ids = match owner {
                Some(owner) => vec![owner.clone()],
                None => list_target_ids(canon, world, actor_id, kind, &ids),
            };
            let command = CommandView {
                id: verb_id.clone(),
                target: kind.as_str().to_string(),
                names: target_ids.iter().map(name_of).collect::<Result<_, _>>()?,
                owner: owner.cloned(),
            };
            if !commands.contains(&command) {
                commands.push(command);
            }
        }
    }
    Ok(commands)
}

/// The ids the actor can name now for a command that takes `kind`: the exits from here, the
/// actors or items among `ids`, or the places they visited.
fn list_target_ids(
    canon: &Canon,
    world: &ecs::World,
    actor_id: &ActorId,
    kind: TargetKind,
    ids: &[NamedId],
) -> Vec<String> {
    let picked: Vec<String> = match kind {
        TargetKind::None => Vec::new(),
        TargetKind::Path => world
            .actors()
            .get::<component::Place>(actor_id)
            .place_id()
            .map(|place| exits_at(canon, world, actor_id, place.as_str()))
            .unwrap_or_default()
            .into_iter()
            .map(|exit| exit.path_id)
            .collect(),
        TargetKind::Actor => ids
            .iter()
            .filter(|named_id| {
                world
                    .actors()
                    .contains(&ActorId::from(named_id.id.as_str()))
            })
            .map(|named_id| named_id.id.clone())
            .collect(),
        TargetKind::Item => ids
            .iter()
            .filter(|named_id| canon.items.contains_key(&named_id.id))
            .map(|named_id| named_id.id.clone())
            .collect(),
        TargetKind::Place => (world.actors().get::<component::Visit>(actor_id).iter())
            .map(|(place, _)| place.to_string())
            .collect(),
    };
    let mut unique = Vec::new();
    for id in picked {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    unique
}

/// `map`: each place in the actor's `visited` that `is_kept` keeps by id, with the exits they saw
/// there last. Where an exit goes, its ticks and whether it is locked are read now.
pub(crate) fn map(
    canon: &Canon,
    world: &ecs::World,
    actor_id: &ActorId,
    is_kept: impl Fn(&str) -> bool,
) -> Result<Vec<PlaceView>, Reason> {
    if !world.actors().contains(actor_id) {
        return Err(Reason::UnknownId);
    }
    let visits = world.actors().get::<component::Visit>(actor_id);
    let kept_visits = visits.iter().filter(|(place_id, _)| is_kept(place_id));
    let places = kept_visits.map(|(place_id, seen_path_ids)| {
        Ok(PlaceView {
            place_id: place_id.to_string(),
            name: find_place_name(canon, place_id).ok_or(Reason::UnknownId)?,
            exits: (canon.paths.exits(place_id))
                .filter(|exit| {
                    seen_path_ids
                        .iter()
                        .any(|path_id| exit.path_def.id == **path_id)
                })
                .map(|exit| exit_view(canon, world, actor_id, exit))
                .collect(),
        })
    });
    places.collect()
}

/// What keeps this actor busy, and until when: `on the stairs until tick 1300` on a walk,
/// `chanting until tick 1300` on any other timed action, read from its verb, and
/// `recovering until tick 1300` in a cooldown.
pub(crate) fn busy_line(canon: &Canon, world: &ecs::World, actor_id: &ActorId) -> Option<String> {
    let act = world.actors().get::<component::Act>(actor_id);
    let act_state = act.get();
    let location = world.actors().get::<component::Place>(actor_id).get()?;
    if !act.is_busy(world.world_time) {
        return None;
    }
    let until = act_state.busy_until?;
    let doing = match (&act_state.pending, location.find_walked_exit(canon)) {
        (Some(_), Some(exit)) => format!("on the {}", exit.name(canon)),
        (Some(pending), None) => gerund(&pending.verb),
        (None, _) => "recovering".to_string(),
    };
    Some(format!("{doing} until tick {until}"))
}

/// `chant` as `chanting`, `strike` as `striking`, and `tie` as `tying`.
fn gerund(verb: &str) -> String {
    if let Some(stem) = verb.strip_suffix("ie") {
        return format!("{stem}ying");
    }
    match verb.strip_suffix('e') {
        Some(stem) if !stem.is_empty() && !stem.ends_with('e') => format!("{stem}ing"),
        _ => format!("{verb}ing"),
    }
}

/// Place prose for this actor's current place.
pub(crate) fn place_prose(canon: &Canon, world: &ecs::World, actor_id: &ActorId) -> String {
    let place = world.actors().get::<component::Place>(actor_id).place_id();
    let Some(place) = place.and_then(|place| canon.places.get(place)) else {
        return String::new();
    };
    place
        .body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The name a player reads for the place `id`, or `None` when the pack has no such place.
pub(crate) fn find_place_name(canon: &Canon, id: &str) -> Option<String> {
    canon.places.get(id).map(|place| place.name.clone())
}
