//! What one viewer may hear from a round.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::entity;
use crate::intent::Viewer;
use crate::rules::effect::{Effect, EffectKind};
use crate::state::change::{self, Change};
use crate::state::id::{ActorId, PlaceId};
use crate::state::State;

pub(crate) fn can_see_fact(canon: &Canon, state: &State, viewer: &Viewer, fact_id: &str) -> bool {
    let actor_id = viewer.actor_id();
    let Some(place) = state.actors().place.get(actor_id) else {
        return false;
    };
    let known = state.actors().known_facts.get(actor_id);
    if known.is_some_and(|facts| facts.iter().any(|id| id == fact_id)) {
        return true;
    }
    let Some(fact) = canon.facts.get(fact_id) else {
        return false;
    };
    !fact.hidden && fact.place_id == *place
}

/// Who is asking.
pub(crate) enum Recipient {
    /// The referee sees every change.
    Gm,
    Actor(ActorId),
}

/// The changes and narrate lines for one seat.
pub(crate) struct Delivery {
    pub changes: Vec<String>,
    /// One plain line of the meter and item changes this seat may know.
    pub line: String,
    pub narrations: Vec<String>,
}

/// One narrate line addressed to one person. Returned from the engine's round to the host
/// table, which sends it to that one player.
pub struct RecipientLine {
    pub to: ActorId,
    pub text: String,
}

/// The change keys one actor can see, and the line they are shown. Returned from the engine's
/// round to the host table, which sends the line to that actor's player.
pub struct ViewerChanges {
    pub viewer: ActorId,
    pub changes: Vec<String>,
    pub line: String,
}

/// Changes and narrate lines this seat can receive.
pub(crate) fn show_to(
    canon: &Canon,
    state: &State,
    seat: &Recipient,
    changes: &[Change],
    effects: &[Effect],
) -> Delivery {
    let actor = match seat {
        Recipient::Gm => None,
        Recipient::Actor(id) => Some(id),
    };
    let visible: Vec<Change> = changes
        .iter()
        .filter(|change| actor.is_none_or(|id| can_see(canon, state, id, change)))
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
    Delivery {
        changes: keys.into_iter().collect(),
        line: spoken.join(" "),
        narrations: narrations(canon, state, effects, actor),
    }
}

/// One sentence for a meter or item change this viewer may know.
fn sentence(
    canon: &Canon,
    viewer: &ActorId,
    change: &Change,
    changes: &[Change],
) -> Option<String> {
    match &change.key {
        change::Key::ActorMeter(actor, key) => {
            let whose = possessive_for(canon, viewer, actor);
            meter_sentence(
                canon,
                &entity::Id::Actor(actor.clone().into()),
                key,
                &whose,
                &change.after,
            )
        }
        change::Key::WorldMeter(key) => meter_sentence(
            canon,
            &entity::Id::Global,
            key,
            "The world's",
            &change.after,
        ),
        change::Key::StoryMeter(story, key) => {
            let id = entity::Id::Story(story.clone().into());
            let whose = format!("The {story} story's");
            meter_sentence(canon, &id, key, &whose, &change.after)
        }
        change::Key::PlaceMeter(place, key) => {
            let id = entity::Id::Place(place.clone().into());
            let whose = format!("{}'s", crate::rules::lines::place_name(canon, place));
            meter_sentence(canon, &id, key, &whose, &change.after)
        }
        change::Key::ActorItem(actor, id) => {
            actor_item_sentence(canon, viewer, actor, id, change, changes)
        }
        change::Key::PlaceItem(_, id) => place_item_sentence(canon, id, change, changes),
        _ => None,
    }
}

fn meter_sentence(
    canon: &Canon,
    id: &entity::Id,
    key: &str,
    whose: &str,
    after: &change::Value,
) -> Option<String> {
    if canon
        .meter_def(id.as_ref(), key)
        .is_none_or(|def| def.hidden)
    {
        return None;
    }
    let change::Value::Number(value) = after else {
        return None;
    };
    Some(format!("{whose} {key} is now {value}."))
}

/// A change to what one actor carries.
fn actor_item_sentence(
    canon: &Canon,
    viewer: &ActorId,
    actor: &str,
    id: &str,
    change: &Change,
    changes: &[Change],
) -> Option<String> {
    let (before, after) = counts(change)?;
    let name = canon.display_name(id);
    let who = subject_for(canon, viewer, actor);
    let owner = possessive_for(canon, viewer, actor);
    let sentence = match (after < before, other_side(changes, change, id)) {
        (true, Some(entity::Id::Actor(_))) => format!("{who} handed over the {name}."),
        (true, Some(_)) => format!("{who} dropped the {name}."),
        (false, Some(entity::Id::Actor(_))) => format!("{who} received the {name}."),
        (false, Some(_)) => format!("{who} picked up the {name}."),
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
    if matches!(other_side(changes, change, id), Some(entity::Id::Actor(_))) {
        return None;
    }
    let name = canon.display_name(id);
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

/// Where the item went to or came from, among changes this viewer can see.
fn other_side(changes: &[Change], change: &Change, id: &str) -> Option<entity::Id> {
    let (before, after) = counts(change)?;
    let down = after < before;
    changes.iter().find_map(|other| {
        let (other_before, other_after) = counts(other)?;
        if (other_after < other_before) == down {
            return None;
        }
        match &other.key {
            change::Key::ActorItem(actor, item) if item == id => {
                Some(entity::Id::Actor(actor.clone().into()))
            }
            change::Key::PlaceItem(place, item) if item == id => {
                Some(entity::Id::Place(place.clone().into()))
            }
            _ => None,
        }
    })
}

/// "You" for the viewer, the display name for anyone else.
fn subject_for(canon: &Canon, viewer: &ActorId, actor: &str) -> String {
    if *viewer == *actor {
        "You".to_string()
    } else {
        canon.display_name(actor)
    }
}

fn possessive_for(canon: &Canon, viewer: &ActorId, actor: &str) -> String {
    if *viewer == *actor {
        "Your".to_string()
    } else {
        format!("{}'s", canon.display_name(actor))
    }
}

fn narrations(
    canon: &Canon,
    state: &State,
    effects: &[Effect],
    viewer: Option<&ActorId>,
) -> Vec<String> {
    let mut lines = Vec::new();
    for effect in effects {
        let EffectKind::Narrate { to, fact_ids, text } = &effect.kind else {
            continue;
        };
        let Some(id) = viewer else {
            lines.push(text.clone());
            continue;
        };
        if to.iter().any(|name| name == id) && can_see_facts(canon, state, id, fact_ids) {
            lines.push(text.clone());
        }
    }
    lines
}

fn can_see_facts(canon: &Canon, state: &State, viewer: &ActorId, fact_ids: &[String]) -> bool {
    let Some(seat) = state.viewer_of(viewer) else {
        return false;
    };
    fact_ids
        .iter()
        .all(|fact| can_see_fact(canon, state, &seat, fact))
}

fn can_see(canon: &Canon, state: &State, viewer: &ActorId, change: &Change) -> bool {
    if !state.actors().kind.contains_key(viewer) {
        return false;
    }
    match &change.key {
        // Flags are rules bookkeeping: the GM narrates what one means.
        change::Key::WorldFlag(_)
        | change::Key::ActorFlag(_, _)
        | change::Key::PlaceFlag(_, _)
        | change::Key::StoryFlag(_, _)
        | change::Key::ExitOpen(_, _)
        | change::Key::LastActRound(_)
        | change::Key::IdleActsThisWindow(_)
        | change::Key::LiveActsThisWindow(_) => false,
        change::Key::WorldMeter(key) => is_shown(canon, &entity::Id::Global, key),
        change::Key::StoryMeter(story, key) => {
            is_shown(canon, &entity::Id::Story(story.clone().into()), key)
        }
        change::Key::PlaceMeter(place, key) => {
            place_of(state, viewer) == Some(place.as_str())
                && is_shown(canon, &entity::Id::Place(place.clone().into()), key)
        }
        change::Key::ActorKnownFact(actor) | change::Key::ActorMeter(actor, _) => {
            *viewer == **actor
        }
        change::Key::ActorModifier(actor, id) => {
            *viewer == **actor && canon.modifiers.get(id).is_some_and(|def| !def.hidden)
        }
        change::Key::PlaceItem(place, _) => place_of(state, viewer) == Some(place.as_str()),
        change::Key::Note(id) => can_see_note(canon, state, viewer, id),
        change::Key::ActorPlace(actor) => {
            *viewer == **actor
                || same_place(state, viewer, actor)
                || is_at(state, viewer, &change.before)
                || is_at(state, viewer, &change.after)
        }
        change::Key::ActorBusy(actor)
        | change::Key::ActorItem(actor, _)
        | change::Key::ActorWielded(actor) => {
            *viewer == **actor || same_place(state, viewer, actor)
        }
    }
}

/// Whether the pack shows meter `key` on `id` to players.
fn is_shown(canon: &Canon, id: &entity::Id, key: &str) -> bool {
    canon
        .meter_def(id.as_ref(), key)
        .is_some_and(|def| !def.hidden)
}

fn place_of<'a>(state: &'a State, viewer: &ActorId) -> Option<&'a str> {
    state.actors().place.get(viewer).map(String::as_str)
}

fn same_place(state: &State, viewer: &ActorId, actor: &str) -> bool {
    let actor_place = state.actors().place.get(actor).map(String::as_str);
    match (place_of(state, viewer), actor_place) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn is_at(state: &State, viewer: &ActorId, value: &change::Value) -> bool {
    match (place_of(state, viewer), value) {
        (Some(place), change::Value::Text(text)) => place == text,
        _ => false,
    }
}

fn can_see_note(canon: &Canon, state: &State, viewer: &ActorId, id: &str) -> bool {
    let Some(note) = state.notes().iter().find(|note| note.id == id) else {
        return false;
    };
    let in_room = note.place.as_ref().map(PlaceId::as_str) == place_of(state, viewer);
    let addressed = note.to.iter().any(|name| name == viewer);
    if !in_room && !addressed {
        return false;
    }
    let Some(seat) = state.viewer_of(viewer) else {
        return false;
    };
    note.fact_ids
        .iter()
        .all(|fact| can_see_fact(canon, state, &seat, fact))
}
