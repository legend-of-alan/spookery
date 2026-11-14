//! What one viewer may hear from a round.

use std::collections::BTreeSet;

use spookery_protocol::{ChangeValue, ChangeView};

use crate::canon::Canon;
use crate::entity;
use crate::intent::Viewer;
use crate::rules::effect::{Effect, EffectKind};
use crate::state::change::{self, Change};
use crate::state::id::{ActorId, PlaceId};
use crate::state::{component, State};

pub(crate) fn can_see_fact(canon: &Canon, state: &State, viewer: &Viewer, fact_id: &str) -> bool {
    let actor_id = viewer.actor_id();
    let Some(place) = state.actors().get::<component::Place>(actor_id).get() else {
        return false;
    };
    let known = state.actors().get::<component::KnownFacts>(actor_id).get();
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
    pub changed_keys: Vec<String>,
    /// The changes this seat may know, as its player is sent them.
    pub changes: Vec<ChangeView>,
    pub narrations: Vec<String>,
}

/// One narrate line addressed to one person. Returned from the engine's round to the host
/// table, which sends it to that one player.
pub struct RecipientLine {
    pub to: ActorId,
    pub text: String,
}

/// The change keys one actor can see, and the changes they are shown. Returned
/// from the engine's round to the host table, which sends the changes to that actor's player.
pub struct ViewerChanges {
    pub viewer: ActorId,
    pub changed_keys: Vec<String>,
    pub changes: Vec<ChangeView>,
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
    let mut views = Vec::new();
    for change in &visible {
        keys.insert(change.key.to_string());
        if actor.is_some() {
            views.extend(to_change_view(canon, change));
        }
    }
    Delivery {
        changed_keys: keys.into_iter().collect(),
        changes: views,
        narrations: narrations(canon, state, effects, actor),
    }
}

/// The entry a player is sent for a change they may know, or none while its value has no wire
/// meaning: a modifier's and busy's values are the journal's.
fn to_change_view(canon: &Canon, change: &Change) -> Option<ChangeView> {
    if matches!(
        change.key,
        change::Key::ActorModifier(..) | change::Key::ActorBusy(_)
    ) {
        return None;
    }
    let (component, entity, key) = change.key.to_parts();
    let entity_name = match &entity {
        entity::Id::Actor(id) => canon.display_name(id.as_str()),
        entity::Id::Place(id) => crate::rules::lines::place_name(canon, id.as_str()),
        entity::Id::Story(id) => id.to_string(),
        entity::Id::Global => String::new(),
    };
    let name = match (&change.key, &change.after) {
        (change::Key::ActorPlace(_), change::Value::Text(place)) => {
            crate::rules::lines::place_name(canon, place)
        }
        _ => canon.display_name(key),
    };
    Some(ChangeView {
        component: component.to_string(),
        entity: entity.to_string(),
        entity_name,
        key: key.to_string(),
        name: if name == key { String::new() } else { name },
        before: to_change_value(&change.before),
        after: to_change_value(&change.after),
    })
}

/// A watched value as the wire writes it: none when the key was absent.
fn to_change_value(value: &change::Value) -> Option<ChangeValue> {
    Some(match value {
        change::Value::Empty => return None,
        change::Value::Text(text) => ChangeValue::Text(text.clone()),
        change::Value::Number(number) => ChangeValue::Number(*number),
        change::Value::Bool(mark) => ChangeValue::Bool(*mark),
        change::Value::List(ids) => ChangeValue::List(ids.clone()),
    })
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
    if !state.actors().contains(viewer) {
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
        change::Key::ActorMeter(actor, key) => {
            *viewer == **actor && is_shown(canon, &entity::Id::Actor(actor.clone().into()), key)
        }
        change::Key::ActorKnownFact(actor) => *viewer == **actor,
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

/// Where `viewer` stands. The place is borrowed for as long as both the save and the id are.
fn place_of<'a>(state: &'a State, viewer: &'a ActorId) -> Option<&'a str> {
    state.actors().get::<component::Place>(viewer).get()
}

fn same_place(state: &State, viewer: &ActorId, actor: &str) -> bool {
    // The actor comes out of a change key as text, so it is read under its id.
    let actor = ActorId::from(actor);
    let actor_place = state.actors().get::<component::Place>(&actor).get();
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
