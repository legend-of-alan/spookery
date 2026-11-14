//! What one viewer may hear from a round.

use std::collections::BTreeSet;

use spookery_protocol::{self as protocol, ChangeView};

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
    pub changes: Vec<String>,
    /// The meter and item changes this seat may know.
    pub entries: Vec<ChangeView>,
    pub narrations: Vec<String>,
}

/// One narrate line addressed to one person. Returned from the engine's round to the host
/// table, which sends it to that one player.
pub struct RecipientLine {
    pub to: ActorId,
    pub text: String,
}

/// The change keys one actor can see, and the meter and item changes they are shown. Returned
/// from the engine's round to the host table, which sends the entries to that actor's player.
pub struct ViewerChanges {
    pub viewer: ActorId,
    pub changes: Vec<String>,
    pub entries: Vec<ChangeView>,
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
    let mut entries = Vec::new();
    for change in &visible {
        keys.insert(change.key.to_string());
        if actor.is_some() {
            entries.extend(to_change_view(canon, change));
        }
    }
    Delivery {
        changes: keys.into_iter().collect(),
        entries,
        narrations: narrations(canon, state, effects, actor),
    }
}

/// The entry for a change a player may know when it is a meter that reads and is not hidden, or
/// an item whose count moved.
fn to_change_view(canon: &Canon, change: &Change) -> Option<ChangeView> {
    let (component, entity, key) = match &change.key {
        change::Key::ActorMeter(actor, key) => (
            protocol::Component::Meter,
            entity::Id::Actor(actor.clone().into()),
            key,
        ),
        change::Key::PlaceMeter(place, key) => (
            protocol::Component::Meter,
            entity::Id::Place(place.clone().into()),
            key,
        ),
        change::Key::StoryMeter(story, key) => (
            protocol::Component::Meter,
            entity::Id::Story(story.clone().into()),
            key,
        ),
        change::Key::WorldMeter(key) => (protocol::Component::Meter, entity::Id::Global, key),
        change::Key::ActorItem(actor, id) => (
            protocol::Component::Item,
            entity::Id::Actor(actor.clone().into()),
            id,
        ),
        change::Key::PlaceItem(place, id) => (
            protocol::Component::Item,
            entity::Id::Place(place.clone().into()),
            id,
        ),
        _ => return None,
    };
    let (before, after) = (change.before.as_number(), change.after.as_number()?);
    let (shown, name) = match component {
        protocol::Component::Meter => (is_shown(canon, &entity, key), String::new()),
        protocol::Component::Item => (
            before.is_some_and(|before| before != after),
            canon.display_name(key),
        ),
    };
    if !shown {
        return None;
    }
    let entity_name = match &entity {
        entity::Id::Actor(id) => canon.display_name(id.as_str()),
        entity::Id::Place(id) => crate::rules::lines::place_name(canon, id.as_str()),
        entity::Id::Story(id) => id.to_string(),
        entity::Id::Global => String::new(),
    };
    Some(ChangeView {
        component,
        entity: entity.to_string(),
        entity_name,
        key: key.clone(),
        name,
        before,
        after,
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
