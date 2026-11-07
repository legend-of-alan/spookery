//! The place name, place prose and busy line a view carries, and a player's help and map.

use spookery_protocol::{PlaceView, Seat, WordView, WORDS};

use crate::canon::Canon;
use crate::intent::{Target, Viewer};
use crate::model::{ActorLive, Presence, State};
use crate::rules::cond;
use crate::rules::gate::{nameable, verb_for};
use crate::rules::rule::{Owner, TargetKind, When};
use crate::rules::travel::{exit_stood_on, exit_view, exits_at};
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
    let actor = state
        .actors()
        .records()
        .get(actor_id)
        .ok_or(Reason::UnknownId)?;
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
                    None => names_for(canon, state, actor_id, actor, kind, &names),
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
    actor_id: &str,
    actor: &ActorLive,
    kind: TargetKind,
    nameable: &[String],
) -> Vec<String> {
    let picked: Vec<String> = match kind {
        TargetKind::None => Vec::new(),
        TargetKind::Exit => canon
            .places
            .get(&actor.place)
            .map(|place| exits_at(canon, state, actor_id, place))
            .unwrap_or_default()
            .into_iter()
            .map(|exit| exit.name)
            .collect(),
        TargetKind::Actor => nameable
            .iter()
            .filter(|name| state.actors().records().contains_key(name))
            .cloned()
            .collect(),
        TargetKind::Item => nameable
            .iter()
            .filter(|name| canon.items.contains_key(*name))
            .cloned()
            .collect(),
        TargetKind::Place => actor.visited.keys().cloned().collect(),
    };
    let mut names = Vec::new();
    for name in picked {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// `map`: each place in the actor's `visited`, with the exits they saw there last. Where an
/// exit goes, its ticks and whether it is locked are read now.
pub(crate) fn map(canon: &Canon, state: &State, actor_id: &str) -> Result<Vec<PlaceView>, Reason> {
    let actor = state
        .actors()
        .records()
        .get(actor_id)
        .ok_or(Reason::UnknownId)?;
    let places = actor.visited.iter().map(|(id, seen)| PlaceView {
        name: place_name(canon, id),
        exits: canon
            .places
            .get(id)
            .map(|place| {
                place
                    .exits
                    .iter()
                    .filter(|exit| seen.contains(&exit.name))
                    .map(|exit| exit_view(canon, state, exit))
                    .collect()
            })
            .unwrap_or_default(),
    });
    Ok(places.collect())
}

/// What keeps this actor busy, and until when: `on the stairs until tick 1300` on a walk,
/// `chanting until tick 1300` on any other timed action, read from its verb, and
/// `recovering until tick 1300` in a cooldown.
pub(crate) fn busy_line(canon: &Canon, state: &State, actor_id: &str) -> Option<String> {
    let actor = state.actors().records().get(actor_id)?;
    if !actor.is_busy(state.world_time) {
        return None;
    }
    let until = actor.act.busy_until?;
    let doing = match (&actor.act.pending, exit_stood_on(canon, &actor.place)) {
        (Some(_), Some(exit)) => format!("on the {}", exit.name),
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
pub(crate) fn place_prose(canon: &Canon, state: &State, actor_id: &str) -> String {
    let Some(actor) = state.actors().records().get(actor_id) else {
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

/// The place's display name, the exit's name for a walker on one, or else the id.
pub(crate) fn place_name(canon: &Canon, id: &str) -> String {
    if let Some(exit) = exit_stood_on(canon, id) {
        return exit.name.clone();
    }
    canon
        .places
        .get(id)
        .map_or_else(|| id.to_string(), |place| place.display_name.clone())
}
