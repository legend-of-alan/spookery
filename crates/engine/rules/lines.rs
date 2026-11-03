//! The place name, place prose and busy line a view carries.

use crate::canon::Canon;
use crate::model::{PendingKind, State};
use crate::model::{MINUTES_PER_DAY, MINUTES_PER_HOUR};

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
