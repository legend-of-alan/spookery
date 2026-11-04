//! The place name, place prose and busy line a view carries, and a player's help and map.

use spookery_protocol::{PlaceView, Seat, WordView, WORDS};

use crate::canon::Canon;
use crate::intent::{Target, Viewer};
use crate::model::{PendingKind, Presence, State};
use crate::model::{MINUTES_PER_DAY, MINUTES_PER_HOUR};
use crate::rules::cond;
use crate::rules::gate::{nameable, verb_for};
use crate::rules::rule::{Owner, TargetKind, When};
use crate::rules::travel::exits_at;
use crate::Reason;

/// `help`: the built-in player words but `claim`, then each pack verb `verb_for` reaches
/// now and whose `if` holds. A cast member's or item's verb is reached by naming them
/// (`nameable`), so it carries that owner as its one name. Cost is not checked.
pub(crate) fn help(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    actor_id: &str,
) -> Result<Vec<WordView>, Reason> {
    if !state.actors.contains_key(actor_id) {
        return Err(Reason::UnknownId);
    }
    let viewer = Viewer::Player(actor_id.to_string());
    let mut words: Vec<WordView> = WORDS
        .iter()
        .filter(|word| word.seat == Seat::Player && word.name != "claim")
        .map(|word| WordView {
            name: word.name.to_string(),
            target: TargetKind::None.as_str().to_string(),
            names: Vec::new(),
            owner: None,
        })
        .collect();
    let names = nameable(canon, state, presence, &viewer);
    let hints = std::iter::once(None).chain(names.iter().map(|name| Some(name.as_str())));
    let hints: Vec<Option<&str>> = hints.collect();
    for verb in canon.verbs() {
        for &hint in &hints {
            let Some(rule) = verb_for(canon, state, actor_id, &verb, hint) else {
                continue;
            };
            // A rule reached by naming its owner carries that owner. Any other is bare.
            let (owner, target) = match (&rule.owner, hint) {
                (_, None) => (None, Target::Empty),
                (Owner::Cast(id), Some(hint)) if id == hint => {
                    (Some(id), Target::Actor(id.clone()))
                }
                (Owner::Item(id), Some(hint)) if id == hint => (Some(id), Target::Item(id.clone())),
                _ => continue,
            };
            if !cond::if_holds(canon, state, presence, &viewer, &verb, target, &rule.if_) {
                continue;
            }
            let kind = match &rule.when {
                When::Verb { target, .. } => *target,
                _ => TargetKind::None,
            };
            let word = WordView {
                name: verb.clone(),
                target: kind.as_str().to_string(),
                names: match owner {
                    Some(owner) => vec![owner.clone()],
                    None => names_for(canon, state, presence, actor_id, kind, &names),
                },
                owner: owner.cloned(),
            };
            if !words.contains(&word) {
                words.push(word);
            }
        }
    }
    Ok(words)
}

/// What the actor can name now for a word that takes `kind`: the exits from here, the
/// actors or items among `nameable`, or the places they visited.
fn names_for(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    actor_id: &str,
    kind: TargetKind,
    nameable: &[String],
) -> Vec<String> {
    let Some(actor) = state.actors.get(actor_id) else {
        return Vec::new();
    };
    let picked: Vec<String> = match kind {
        TargetKind::None => Vec::new(),
        TargetKind::Exit => match canon.places.get(&actor.place) {
            Some(place) => exits_at(canon, state, presence, actor_id, place)
                .into_iter()
                .map(|exit| exit.name)
                .collect(),
            None => Vec::new(),
        },
        TargetKind::Actor => nameable
            .iter()
            .filter(|name| state.actors.contains_key(*name))
            .cloned()
            .collect(),
        TargetKind::Item => nameable
            .iter()
            .filter(|name| canon.items.contains_key(*name))
            .cloned()
            .collect(),
        TargetKind::Place => actor.visited.clone(),
    };
    let mut names = Vec::new();
    for name in picked {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// `map`: each place in the actor's `visited`, with the exits they may see from there.
pub(crate) fn map(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    actor_id: &str,
) -> Result<Vec<PlaceView>, Reason> {
    let actor = state.actors.get(actor_id).ok_or(Reason::UnknownId)?;
    let places = actor.visited.iter().map(|id| PlaceView {
        name: place_name(canon, id),
        exits: canon
            .places
            .get(id)
            .map(|place| exits_at(canon, state, presence, actor_id, place))
            .unwrap_or_default(),
    });
    Ok(places.collect())
}

/// `on the stairs until 21:40` while a journey is still running.
pub(crate) fn busy_line(state: &State, actor_id: &str) -> Option<String> {
    let actor = state.actors.get(actor_id)?;
    if !actor.is_busy(state.world_time) {
        return None;
    }
    let until = actor.act.busy_until?;
    let via = match actor.act.pending.as_ref().map(|pending| &pending.kind) {
        Some(PendingKind::Arrive { via, .. }) if !via.is_empty() => via,
        _ => return None,
    };
    Some(format!("on the {via} until {}", time_of_day(until)))
}

/// Place prose for this actor's current place.
pub(crate) fn place_prose(canon: &Canon, state: &State, actor_id: &str) -> String {
    let Some(actor) = state.actors.get(actor_id) else {
        return String::new();
    };
    let Some(place) = canon.places.get(&actor.place) else {
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

/// Minutes from midnight as `21:40`.
fn time_of_day(minutes: u64) -> String {
    let hour = (minutes % MINUTES_PER_DAY) / MINUTES_PER_HOUR;
    let minute = minutes % MINUTES_PER_HOUR;
    format!("{hour:02}:{minute:02}")
}

pub(crate) fn place_name(canon: &Canon, id: &str) -> String {
    canon
        .places
        .get(id)
        .map_or_else(|| id.to_string(), |place| place.display_name.clone())
}
