//! What one viewer may hear from a round.

use std::collections::BTreeSet;

use spookery_protocol::ChangeView;

use crate::canon::Canon;
use crate::ecs::change::{Change, ChangeValue};
use crate::ecs::component::{
    Act, Component, Item, KnownFacts, Meter, Modifier, Note, Place, WieldedItem,
};
use crate::ecs::id::{ActorId, GlobalId, PlaceId};
use crate::ecs::{self, component};
use crate::entity;
use crate::intent::Viewer;
use crate::rules::effect::{Effect, EffectKind};

pub(crate) fn can_see_fact(
    canon: &Canon,
    world: &ecs::World,
    viewer: &Viewer,
    fact_id: &str,
) -> bool {
    let actor_id = viewer.actor_id();
    let Some(place) = world.actors().get::<component::Place>(actor_id).get() else {
        return false;
    };
    let known = world.actors().get::<component::KnownFacts>(actor_id).get();
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
    world: &ecs::World,
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
        .filter(|change| actor.is_none_or(|id| can_see(canon, world, id, change)))
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
        narrations: narrations(canon, world, effects, actor),
    }
}

/// The entry a player is sent for a change they may know, or none while its value has no wire
/// meaning: a modifier's and busy's values are the journal's.
fn to_change_view(canon: &Canon, change: &Change) -> Option<ChangeView> {
    let key = &change.key;
    if [Modifier::ID, Act::BUSY].contains(&key.component_id) {
        return None;
    }
    // A name the pack lacks stays empty, and the wire leaves it out.
    let entity_name = match &key.entity_id {
        entity::Id::Actor(id) => canon.name(id.as_str()).map(str::to_string),
        entity::Id::Place(id) => crate::rules::lines::find_place_name(canon, id.as_str()),
        entity::Id::Story(id) => canon.name(id.as_str()).map(str::to_string),
        entity::Id::Global => None,
    };
    let entry_id = key.entry_id.clone().unwrap_or_default();
    let entry_name = match (key.component_id, &change.after) {
        (Place::ID, Some(ChangeValue::Text(place))) => {
            crate::rules::lines::find_place_name(canon, place)
        }
        (Item::ID, _) => canon.name(&entry_id).map(str::to_string),
        _ => None,
    };
    Some(ChangeView {
        component_id: key.component_id.to_string(),
        entity_id: key.entity_id.to_string(),
        entity_name: entity_name.unwrap_or_default(),
        entry_name: entry_name.unwrap_or_default(),
        entry_id,
        before: change.before.clone(),
        after: change.after.clone(),
    })
}

fn narrations(
    canon: &Canon,
    world: &ecs::World,
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
        if to.iter().any(|actor_id| actor_id == id) && can_see_facts(canon, world, id, fact_ids) {
            lines.push(text.clone());
        }
    }
    lines
}

fn can_see_facts(canon: &Canon, world: &ecs::World, viewer: &ActorId, fact_ids: &[String]) -> bool {
    let Some(seat) = (world.actors().get::<component::ControlType>(viewer)).viewer() else {
        return false;
    };
    fact_ids
        .iter()
        .all(|fact| can_see_fact(canon, world, &seat, fact))
}

fn can_see(canon: &Canon, world: &ecs::World, viewer: &ActorId, change: &Change) -> bool {
    if !world.actors().contains(viewer) {
        return false;
    }
    let key = &change.key;
    let entry = key.entry_id.as_deref().unwrap_or_default();
    match (key.component_id, &key.entity_id) {
        (Meter::ID, entity::Id::Global | entity::Id::Story(_)) => {
            is_shown(canon, &key.entity_id, entry)
        }
        (Meter::ID, entity::Id::Place(place)) => {
            place_of(world, viewer) == Some(place.as_str())
                && is_shown(canon, &key.entity_id, entry)
        }
        (Meter::ID, entity::Id::Actor(actor)) => {
            viewer == actor && is_shown(canon, &key.entity_id, entry)
        }
        (KnownFacts::ID, entity::Id::Actor(actor)) => viewer == actor,
        (Modifier::ID, entity::Id::Actor(actor)) => {
            viewer == actor && canon.modifiers.get(entry).is_some_and(|desc| !desc.hidden)
        }
        (Item::ID, entity::Id::Place(place)) => place_of(world, viewer) == Some(place.as_str()),
        (Note::ID, entity::Id::Global) => can_see_note(canon, world, viewer, entry),
        (Place::ID, entity::Id::Actor(actor)) => {
            viewer == actor
                || same_place(world, viewer, actor)
                || is_at(world, viewer, change.before.as_ref())
                || is_at(world, viewer, change.after.as_ref())
        }
        (Act::BUSY | Item::ID | WieldedItem::ID, entity::Id::Actor(actor)) => {
            viewer == actor || same_place(world, viewer, actor)
        }
        // Flags, doors and act counts are rules bookkeeping: the GM narrates what one means.
        _ => false,
    }
}

/// Whether the pack shows meter `key` on `id` to players.
fn is_shown(canon: &Canon, id: &entity::Id, key: &str) -> bool {
    canon
        .meter_desc(id.as_ref(), key)
        .is_some_and(|desc| !desc.hidden)
}

/// Where `viewer` stands. The place is borrowed for as long as both the save and the id are.
fn place_of<'a>(world: &'a ecs::World, viewer: &'a ActorId) -> Option<&'a str> {
    world.actors().get::<component::Place>(viewer).get()
}

fn same_place(world: &ecs::World, viewer: &ActorId, actor: &ActorId) -> bool {
    let actor_place = world.actors().get::<component::Place>(actor).get();
    match (place_of(world, viewer), actor_place) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn is_at(world: &ecs::World, viewer: &ActorId, value: Option<&ChangeValue>) -> bool {
    match (place_of(world, viewer), value) {
        (Some(place), Some(ChangeValue::Text(text))) => place == text,
        _ => false,
    }
}

fn can_see_note(canon: &Canon, world: &ecs::World, viewer: &ActorId, id: &str) -> bool {
    let Some(note) = world.globals().get::<Note>(&GlobalId).get(id) else {
        return false;
    };
    let in_room = note.place_id.as_ref().map(PlaceId::as_str) == place_of(world, viewer);
    let addressed = note.to_ids.contains(viewer);
    if !in_room && !addressed {
        return false;
    }
    let Some(seat) = (world.actors().get::<component::ControlType>(viewer)).viewer() else {
        return false;
    };
    note.fact_ids
        .iter()
        .all(|fact| can_see_fact(canon, world, &seat, fact))
}
