//! What one viewer may hear from a round.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::effect::{Effect, EffectKind};
use crate::intent::Viewer;
use crate::key::{Change, StateKey, StateValue};
use crate::model::State;

pub(crate) fn can_see_fact(canon: &Canon, now: &State, viewer: &Viewer, fact_id: &str) -> bool {
    let Some(actor) = now.actors.get(viewer.actor_id()) else {
        return false;
    };
    if actor.known_facts().iter().any(|id| id == fact_id) {
        return true;
    }
    let Some(fact) = canon.facts.get(fact_id) else {
        return false;
    };
    !fact.hidden && fact.place_id == actor.place()
}

/// Who is asking.
pub enum Recipient {
    /// The referee sees every change.
    Gm,
    Actor(String),
}

/// The changes and narrate lines for one seat.
pub struct Delivery {
    pub changes: Vec<String>,
    /// One plain line of the meter and item changes this seat may know.
    pub line: String,
    pub narrations: Vec<String>,
}

/// Returned from the engine to the host table, which sends it to that one player.
/// One narrate line addressed to one person.
pub struct RecipientLine {
    pub to: String,
    pub text: String,
}

/// The change keys one actor can see, and the line they are shown.
pub struct ViewerChanges {
    pub viewer: String,
    pub changes: Vec<String>,
    pub line: String,
}

/// Changes and narrate lines this seat can receive.
pub fn observe(
    canon: &Canon,
    now: &State,
    seat: &Recipient,
    changes: &[Change],
    effects: &[Effect],
) -> Delivery {
    let actor = match seat {
        Recipient::Gm => None,
        Recipient::Actor(id) => Some(id.as_str()),
    };
    let visible: Vec<Change> = changes
        .iter()
        .filter(|change| actor.is_none_or(|id| can_see(canon, now, id, change)))
        .cloned()
        .collect();
    let mut keys = BTreeSet::new();
    let mut spoken = Vec::new();
    for change in &visible {
        keys.insert(change.key.to_string());
        if let Some(id) = actor {
            if let Some(sentence) = sentence(canon, id, change, &visible) {
                spoken.push(sentence);
            }
        }
    }
    let lines = match actor {
        None => narrations(canon, now, effects, None),
        Some(id) => narrations(canon, now, effects, Some(id)),
    };
    Delivery {
        changes: keys.into_iter().collect(),
        line: spoken.join(" "),
        narrations: lines,
    }
}

/// One sentence for a meter or item change this viewer may know.
fn sentence(canon: &Canon, viewer: &str, change: &Change, changes: &[Change]) -> Option<String> {
    match &change.key {
        StateKey::ActorMeter(actor, key) => {
            meter_sentence(canon, actor, key, viewer, &change.after)
        }
        StateKey::ActorItem(actor, id) => {
            actor_item_sentence(canon, viewer, actor, id, change, changes)
        }
        StateKey::PlaceItem(_, id) => place_item_sentence(canon, id, change, changes),
        _ => None,
    }
}

fn meter_sentence(
    canon: &Canon,
    actor: &str,
    key: &str,
    viewer: &str,
    after: &StateValue,
) -> Option<String> {
    if canon.meters.get(key).is_some_and(|def| def.hidden) {
        return None;
    }
    let StateValue::Number(value) = after else {
        return None;
    };
    let who = whose(canon, viewer, actor);
    Some(format!("{who} {key} is now {value}."))
}

/// A change to what one actor carries.
fn actor_item_sentence(
    canon: &Canon,
    viewer: &str,
    actor: &str,
    id: &str,
    change: &Change,
    changes: &[Change],
) -> Option<String> {
    let (before, after) = counts(change)?;
    let name = item_name(canon, id);
    let who = subject(canon, viewer, actor);
    let owner = whose(canon, viewer, actor);
    let sentence = match (after < before, other_side(changes, change, id)) {
        (true, Some(Holder::Place)) => format!("{who} dropped the {name}."),
        (true, Some(Holder::Actor)) => format!("{who} handed over the {name}."),
        (false, Some(Holder::Place)) => format!("{who} picked up the {name}."),
        (false, Some(Holder::Actor)) => format!("{who} received the {name}."),
        (true, None) if after == 0 => format!("{owner} {name} was used up."),
        (true, None) => format!("{owner} {name} was lost."),
        (false, None) => format!("{owner} {name} was gained."),
    };
    Some(sentence)
}

/// A change to what lies in the viewer's place. A move to or from an actor is told by the actor's sentence.
fn place_item_sentence(
    canon: &Canon,
    id: &str,
    change: &Change,
    changes: &[Change],
) -> Option<String> {
    let (before, after) = counts(change)?;
    if other_side(changes, change, id) == Some(Holder::Actor) {
        return None;
    }
    let name = item_name(canon, id);
    if after < before {
        Some(format!("The {name} here is gone."))
    } else {
        Some(format!("A {name} turned up here."))
    }
}

/// The before and after counts of an item change, when the count changed.
fn counts(change: &Change) -> Option<(i64, i64)> {
    let (before, after) = (change.before.as_number()?, change.after.as_number()?);
    (before != after).then_some((before, after))
}

/// Who holds an item.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Holder {
    Actor,
    Place,
}

/// Where the item went to or came from, among changes this viewer can see.
fn other_side(changes: &[Change], change: &Change, id: &str) -> Option<Holder> {
    let (before, after) = counts(change)?;
    let down = after < before;
    changes.iter().find_map(|other| {
        let (other_before, other_after) = counts(other)?;
        if (other_after < other_before) == down {
            return None;
        }
        match &other.key {
            StateKey::ActorItem(_, item) if item == id => Some(Holder::Actor),
            StateKey::PlaceItem(_, item) if item == id => Some(Holder::Place),
            _ => None,
        }
    })
}

/// "You" for the viewer, the display name for anyone else.
fn subject(canon: &Canon, viewer: &str, actor: &str) -> String {
    if actor == viewer {
        "You".to_string()
    } else {
        canon.display_name(actor)
    }
}

fn whose(canon: &Canon, viewer: &str, actor: &str) -> String {
    if actor == viewer {
        "Your".to_string()
    } else {
        format!("{}'s", canon.display_name(actor))
    }
}

fn item_name(canon: &Canon, id: &str) -> String {
    canon
        .items
        .get(id)
        .map(|item| item.name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.replace('_', " "))
}

fn narrations(canon: &Canon, now: &State, effects: &[Effect], viewer: Option<&str>) -> Vec<String> {
    let mut lines = Vec::new();
    for effect in effects {
        let EffectKind::Narrate { to, fact_ids, text } = &effect.kind else {
            continue;
        };
        let Some(id) = viewer else {
            lines.push(text.clone());
            continue;
        };
        if to.iter().any(|name| name == id) && can_see_facts(canon, now, id, fact_ids) {
            lines.push(text.clone());
        }
    }
    lines
}

fn can_see_facts(canon: &Canon, now: &State, viewer: &str, fact_ids: &[String]) -> bool {
    let Some(seat) = now.actors.get(viewer).map(|actor| actor.viewer(viewer)) else {
        return false;
    };
    fact_ids
        .iter()
        .all(|fact| can_see_fact(canon, now, &seat, fact))
}

fn can_see(canon: &Canon, now: &State, viewer: &str, change: &Change) -> bool {
    if !now.actors.contains_key(viewer) {
        return false;
    }
    match &change.key {
        StateKey::WorldFlag(_) => true,
        StateKey::LastActRound(_) | StateKey::IdleActsToday(_) | StateKey::LiveActsThisHour(_) => {
            false
        }
        StateKey::ActorKnownFact(actor) => actor == viewer,
        StateKey::PlaceItem(place, _) => place_of(now, viewer) == Some(place.as_str()),
        StateKey::Note(id) => can_see_note(canon, now, viewer, id),
        StateKey::ClockValue(id) | StateKey::ClockPaused(id) => is_clock_owner(now, viewer, id),
        StateKey::ActorPlace(actor) => {
            actor == viewer
                || same_place(now, viewer, actor)
                || is_at(now, viewer, &change.before)
                || is_at(now, viewer, &change.after)
        }
        StateKey::ActorFlag(actor, _)
        | StateKey::ActorMeter(actor, _)
        | StateKey::ActorBusy(actor)
        | StateKey::ActorItem(actor, _)
        | StateKey::ActorWielded(actor) => actor == viewer || same_place(now, viewer, actor),
    }
}

fn place_of<'a>(now: &'a State, viewer: &str) -> Option<&'a str> {
    now.actors.get(viewer).map(|actor| actor.place())
}

fn same_place(now: &State, viewer: &str, actor: &str) -> bool {
    match (place_of(now, viewer), place_of(now, actor)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn is_at(now: &State, viewer: &str, value: &StateValue) -> bool {
    match (place_of(now, viewer), value) {
        (Some(place), StateValue::Text(text)) => place == text,
        _ => false,
    }
}

fn is_clock_owner(now: &State, viewer: &str, id: &str) -> bool {
    now.clocks
        .get(id)
        .is_some_and(|clock| clock.owners.iter().any(|owner| owner == viewer))
}

fn can_see_note(canon: &Canon, now: &State, viewer: &str, id: &str) -> bool {
    let Some(note) = now.notes.iter().find(|note| note.id == id) else {
        return false;
    };
    let in_room = note.place.as_deref() == place_of(now, viewer);
    let addressed = note.to.iter().any(|name| name == viewer);
    if !in_room && !addressed {
        return false;
    }
    let Some(seat) = now.actors.get(viewer).map(|actor| actor.viewer(viewer)) else {
        return false;
    };
    note.fact_ids
        .iter()
        .all(|fact| can_see_fact(canon, now, &seat, fact))
}
