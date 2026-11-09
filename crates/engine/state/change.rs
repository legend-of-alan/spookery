//! Change keys a commit watches, and the values they held.

use std::fmt;

use crate::rules::effect::EntityId;

/// One budget identity. The commit diff summary is this type's `Display`,
/// sorted as text. It is not a path inside `save.yaml`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKey {
    ActorPlace(String),
    ActorFlag(String, String),
    ActorMeter(String, String),
    /// An actor's modifier: its stacks and the tick it expires at.
    ActorModifier(String, String),
    ActorBusy(String),
    ActorKnownFact(String),
    LastActRound(String),
    IdleActsThisWindow(String),
    LiveActsThisWindow(String),
    ActorItem(String, String),
    PlaceItem(String, String),
    PlaceMeter(String, String),
    PlaceFlag(String, String),
    StoryMeter(String, String),
    StoryFlag(String, String),
    /// Whether a door is open: the exit's place and its name.
    ExitOpen(String, String),
    ActorWielded(String),
    Note(String),
    WorldFlag(String),
    WorldMeter(String),
}

impl fmt::Display for ChangeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeKey::ActorPlace(id) => write!(f, "actor_place {id}"),
            ChangeKey::ActorFlag(id, key) => write!(f, "actor_flag {id} {key}"),
            ChangeKey::ActorMeter(id, key) => write!(f, "actor_meter {id} {key}"),
            ChangeKey::ActorModifier(id, key) => write!(f, "modifier {id} {key}"),
            ChangeKey::ActorBusy(id) => write!(f, "actor_busy {id}"),
            ChangeKey::ActorKnownFact(id) => write!(f, "actor_known_fact {id}"),
            ChangeKey::LastActRound(id) => write!(f, "last_act_round {id}"),
            ChangeKey::IdleActsThisWindow(id) => write!(f, "idle_acts_this_window {id}"),
            ChangeKey::LiveActsThisWindow(id) => write!(f, "live_acts_this_window {id}"),
            ChangeKey::ActorItem(actor, id) => write!(f, "actor_item {actor} {id}"),
            ChangeKey::PlaceItem(place, id) => write!(f, "place_item {place} {id}"),
            ChangeKey::PlaceMeter(place, key) => write!(f, "place_meter {place} {key}"),
            ChangeKey::PlaceFlag(place, key) => write!(f, "place_flag {place} {key}"),
            ChangeKey::StoryMeter(story, key) => write!(f, "story_meter {story} {key}"),
            ChangeKey::StoryFlag(story, key) => write!(f, "story_flag {story} {key}"),
            ChangeKey::ExitOpen(place, exit) => write!(f, "exit_open {place} {exit}"),
            ChangeKey::ActorWielded(actor) => write!(f, "actor_wielded {actor}"),
            ChangeKey::Note(id) => write!(f, "note {id}"),
            ChangeKey::WorldFlag(key) => write!(f, "world_flag {key}"),
            ChangeKey::WorldMeter(key) => write!(f, "world_meter {key}"),
        }
    }
}

impl ChangeKey {
    /// The key of meter `key` on `entity`. An exit carries no meters.
    pub(crate) fn meter(entity: &EntityId, key: &str) -> Option<ChangeKey> {
        let key = key.to_string();
        match entity {
            EntityId::Actor(id) => Some(ChangeKey::ActorMeter(id.clone(), key)),
            EntityId::Place(id) => Some(ChangeKey::PlaceMeter(id.clone(), key)),
            EntityId::Story(id) => Some(ChangeKey::StoryMeter(id.clone(), key)),
            EntityId::Exit(..) => None,
            EntityId::World => Some(ChangeKey::WorldMeter(key)),
        }
    }

    /// The key of flag `key` on `entity`. An exit's one flag is `open`.
    pub(crate) fn flag(entity: &EntityId, key: &str) -> ChangeKey {
        let key = key.to_string();
        match entity {
            EntityId::Exit(place, exit) => ChangeKey::ExitOpen(place.clone(), exit.clone()),
            EntityId::Actor(id) => ChangeKey::ActorFlag(id.clone(), key),
            EntityId::Place(id) => ChangeKey::PlaceFlag(id.clone(), key),
            EntityId::Story(id) => ChangeKey::StoryFlag(id.clone(), key),
            EntityId::World => ChangeKey::WorldFlag(key),
        }
    }

    /// The actor this key belongs to, when it has one.
    pub fn actor(&self) -> Option<&str> {
        match self {
            ChangeKey::ActorPlace(id)
            | ChangeKey::ActorFlag(id, _)
            | ChangeKey::ActorMeter(id, _)
            | ChangeKey::ActorModifier(id, _)
            | ChangeKey::ActorBusy(id)
            | ChangeKey::ActorKnownFact(id)
            | ChangeKey::LastActRound(id)
            | ChangeKey::IdleActsThisWindow(id)
            | ChangeKey::LiveActsThisWindow(id)
            | ChangeKey::ActorItem(id, _)
            | ChangeKey::ActorWielded(id) => Some(id),
            ChangeKey::PlaceItem(_, _)
            | ChangeKey::PlaceMeter(_, _)
            | ChangeKey::PlaceFlag(_, _)
            | ChangeKey::StoryMeter(_, _)
            | ChangeKey::StoryFlag(_, _)
            | ChangeKey::ExitOpen(_, _)
            | ChangeKey::Note(_)
            | ChangeKey::WorldFlag(_)
            | ChangeKey::WorldMeter(_) => None,
        }
    }
}

/// One watched value. `Empty` means the key was absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeValue {
    Empty,
    Text(String),
    Number(i64),
    Bool(bool),
    List(Vec<String>),
}

impl ChangeValue {
    pub(crate) fn as_text(&self) -> Option<&str> {
        match self {
            ChangeValue::Text(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn as_number(&self) -> Option<i64> {
        match self {
            ChangeValue::Number(number) => Some(*number),
            _ => None,
        }
    }

    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            ChangeValue::Bool(mark) => Some(*mark),
            _ => None,
        }
    }

    /// Whether this is a list that holds `id`.
    pub(crate) fn contains(&self, id: &str) -> bool {
        matches!(self, ChangeValue::List(ids) if ids.iter().any(|listed| listed == id))
    }
}

/// A key that changed, with the value before the change and the value after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub key: ChangeKey,
    pub before: ChangeValue,
    pub after: ChangeValue,
}
