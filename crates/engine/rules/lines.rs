//! The place name, place prose and busy line a view carries, and a player's help and map.

use spookery_protocol::{exit_phrase, exit_section, Seat, WORDS};

use crate::canon::Canon;
use crate::intent::{Target, Viewer};
use crate::model::{PendingKind, Presence, State};
use crate::model::{MINUTES_PER_DAY, MINUTES_PER_HOUR};
use crate::rules::cond;
use crate::rules::gate::{nameable, verb_for};
use crate::rules::perceive::perceive_as;
use crate::rules::rule::Owner;
use crate::rules::travel::exits_at;
use crate::Reason;

/// `help`: the built-in player words but `claim`, then each pack verb `verb_for` reaches
/// now and whose `if` holds, then the exits and the busy line. A cast member's or item's
/// verb is reached by naming them (`nameable`), so it shows as `ask briar`. Cost is not
/// checked.
pub(crate) fn help(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    actor_id: &str,
) -> Result<String, Reason> {
    let viewer = Viewer::Player(actor_id.to_string());
    let view = perceive_as(canon, state, presence, &viewer)?;
    let mut lines: Vec<String> = WORDS
        .iter()
        .filter(|word| word.seat == Seat::Player && word.name != "claim")
        .map(|word| word.name.to_string())
        .collect();
    let names = nameable(canon, state, presence, &viewer);
    let hints = std::iter::once(None).chain(names.iter().map(|name| Some(name.as_str())));
    let hints: Vec<Option<&str>> = hints.collect();
    for verb in canon.verbs() {
        for &hint in &hints {
            let Some(rule) = verb_for(canon, state, actor_id, &verb, hint) else {
                continue;
            };
            // A rule reached by naming its owner shows with that name. Any other shows bare.
            let (line, target) = match (&rule.owner, hint) {
                (_, None) => (verb.clone(), Target::Empty),
                (Owner::Cast(id), Some(hint)) if id == hint => {
                    (format!("{verb} {hint}"), Target::Actor(id.clone()))
                }
                (Owner::Item(id), Some(hint)) if id == hint => {
                    (format!("{verb} {hint}"), Target::Item(id.clone()))
                }
                _ => continue,
            };
            let holds = cond::if_holds(canon, state, presence, &viewer, &verb, target, &rule.if_);
            if holds && !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    lines.push(exit_section(&view.exits));
    if !view.status.is_empty() {
        lines.push(view.status);
    }
    Ok(lines.join("\n"))
}

/// `map`: each place in the actor's `visited`, with the exits they may see from there.
pub(crate) fn map(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    actor_id: &str,
) -> Result<String, Reason> {
    let actor = state.actors.get(actor_id).ok_or(Reason::UnknownId)?;
    let mut lines = Vec::new();
    for id in &actor.visited {
        lines.push(place_name(canon, id));
        let Some(place) = canon.places.get(id) else {
            continue;
        };
        for exit in exits_at(canon, state, presence, actor_id, place) {
            lines.push(format!("  {}", exit_phrase(&exit)));
        }
    }
    Ok(lines.join("\n"))
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
