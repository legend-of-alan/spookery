//! What one viewer may hear from a round.

use std::collections::BTreeSet;

use spookery_protocol::ChangeView;

use crate::canon::Canon;
use crate::ecs::change::{Change, ChangeValue};
use crate::ecs::component::{
    Act, ActorLocation, Component, Item, Meter, Modifier, Note, Place, WieldedItem,
};
use crate::ecs::id::{ActorId, GlobalId, PlaceId};
use crate::ecs::{self, component};
use crate::entity;
use crate::rules::effect::{Effect, EffectKind};
use crate::rules::grammar;

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
    let actor_id = match seat {
        Recipient::Gm => None,
        Recipient::Actor(id) => Some(id),
    };
    let visible: Vec<Change> = changes
        .iter()
        .filter(|change| actor_id.is_none_or(|id| can_see(canon, world, id, change)))
        .cloned()
        .collect();
    let mut keys = BTreeSet::new();
    let mut views = Vec::new();
    for change in &visible {
        keys.insert(change.key.to_string());
        if actor_id.is_some() {
            views.extend(to_change_view(canon, change));
        }
    }
    Delivery {
        changed_keys: keys.into_iter().collect(),
        changes: views,
        narrations: narrations(effects, actor_id),
    }
}

/// The entry a player is sent for a change they may know, or none while its value has no wire
/// meaning: a modifier's and busy's values are the journal's.
fn to_change_view(canon: &Canon, change: &Change) -> Option<ChangeView> {
    let key = &change.key;
    if [Modifier::ID, Act::BUSY].contains(&key.component_id) {
        return None;
    }
    let entity_name = match &key.entity_id {
        entity::Id::Actor(id) => canon.name(id.as_str()).map(str::to_string),
        entity::Id::Place(id) => crate::rules::lines::find_place_name(canon, id.as_str()),
        entity::Id::Path(_) => None,
        entity::Id::Story(id) => canon.name(id.as_str()).map(str::to_string),
        entity::Id::Global => None,
    };
    let entry_id = key.entry_id.clone().unwrap_or_default();
    let entry_name = match (key.component_id, &change.after) {
        (Place::ID, Some(ChangeValue::Text(after_id))) => {
            let before_id = change.before.as_ref().and_then(ChangeValue::as_text);
            find_place_change_name(canon, before_id, after_id)
        }
        (Item::ID, _) => canon.name(&entry_id).map(str::to_string),
        (Meter::ID, _) => {
            (canon.meter_desc(key.entity_id.as_ref(), &entry_id)).map(|desc| desc.name.clone())
        }
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

/// The name a player reads for a `place` change's `after_id`: the place's name, or for
/// `path:<id>` the name of the exit that path leaves `before_id` by.
fn find_place_change_name(
    canon: &Canon,
    before_id: Option<&str>,
    after_id: &str,
) -> Option<String> {
    if let Some(name) = crate::rules::lines::find_place_name(canon, after_id) {
        return Some(name);
    }
    let Ok(grammar::Object::Path(path_id)) = grammar::Object::parse(after_id) else {
        return None;
    };
    let exit = canon.paths.get(&path_id)?.exit_from(before_id?)?;
    Some(exit.name(canon).to_string())
}

fn narrations(effects: &[Effect], viewer: Option<&ActorId>) -> Vec<String> {
    let mut lines = Vec::new();
    for effect in effects {
        let EffectKind::Narrate { to, text } = &effect.kind else {
            continue;
        };
        if viewer.is_none_or(|id| to.contains(id)) {
            lines.push(text.clone());
        }
    }
    lines
}

fn can_see(canon: &Canon, world: &ecs::World, viewer_id: &ActorId, change: &Change) -> bool {
    if !world.actors().contains(viewer_id) {
        return false;
    }
    let key = &change.key;
    let entry = key.entry_id.as_deref().unwrap_or_default();
    match (key.component_id, &key.entity_id) {
        (Meter::ID, entity::Id::Global | entity::Id::Story(_)) => {
            is_shown(canon, &key.entity_id, entry)
        }
        (Meter::ID, entity::Id::Place(place)) => {
            place_of(world, viewer_id) == Some(place.as_str())
                && is_shown(canon, &key.entity_id, entry)
        }
        (Meter::ID, entity::Id::Actor(actor_id)) => {
            viewer_id == actor_id && is_shown(canon, &key.entity_id, entry)
        }
        (Modifier::ID, entity::Id::Actor(actor_id)) => {
            viewer_id == actor_id && canon.modifiers.get(entry).is_some_and(|desc| !desc.hidden)
        }
        (Item::ID, entity::Id::Place(place)) => place_of(world, viewer_id) == Some(place.as_str()),
        (Note::ID, entity::Id::Global) => can_see_note(world, viewer_id, entry),
        (Place::ID, entity::Id::Actor(actor_id)) => {
            viewer_id == actor_id
                || same_place(world, viewer_id, actor_id)
                || is_at(world, viewer_id, change.before.as_ref())
                || is_at(world, viewer_id, change.after.as_ref())
        }
        (Act::BUSY | Item::ID | WieldedItem::ID, entity::Id::Actor(actor_id)) => {
            viewer_id == actor_id || same_place(world, viewer_id, actor_id)
        }
        _ => false,
    }
}

/// Whether the pack shows meter `key` on `id` to players.
fn is_shown(canon: &Canon, id: &entity::Id, key: &str) -> bool {
    canon
        .meter_desc(id.as_ref(), key)
        .is_some_and(|desc| !desc.hidden)
}

/// The place `viewer_id` stands in, `None` while they walk. The place is borrowed for as long as
/// both the save and the id are.
fn place_of<'a>(world: &'a ecs::World, viewer_id: &'a ActorId) -> Option<&'a str> {
    let place = world.actors().get::<component::Place>(viewer_id).place_id();
    place.map(PlaceId::as_str)
}

/// Where `actor_id` stands. The location is borrowed for as long as both the save and the id
/// are.
fn location_of<'a>(world: &'a ecs::World, actor_id: &'a ActorId) -> Option<&'a ActorLocation> {
    world.actors().get::<component::Place>(actor_id).get()
}

/// The viewer stands where `actor_id` does: in one place, or on one path either way.
fn same_place(world: &ecs::World, viewer_id: &ActorId, actor_id: &ActorId) -> bool {
    match (location_of(world, viewer_id), location_of(world, actor_id)) {
        (Some(viewer), Some(actor)) => viewer.is_with(actor),
        _ => false,
    }
}

fn is_at(world: &ecs::World, viewer_id: &ActorId, change_side: Option<&ChangeValue>) -> bool {
    match (location_of(world, viewer_id), change_side) {
        (Some(location), Some(ChangeValue::Text(location_text))) => location.is_at(location_text),
        _ => false,
    }
}

fn can_see_note(world: &ecs::World, viewer_id: &ActorId, id: &str) -> bool {
    let note = world.globals().get::<Note>(&GlobalId).get(id);
    note.is_some_and(|note| note.is_read_by(viewer_id, place_of(world, viewer_id)))
}
