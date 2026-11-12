//! Change keys a commit watches, and the values they held.

use std::fmt;

use crate::state::component::Meter;
use crate::state::entity;
use crate::Reason;

/// One budget identity. The commit diff summary is this type's `Display`,
/// sorted as text. It is not a path inside `save.yaml`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
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

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::ActorPlace(id) => write!(f, "actor_place {id}"),
            Key::ActorFlag(id, key) => write!(f, "actor_flag {id} {key}"),
            Key::ActorMeter(id, key) => write!(f, "actor_meter {id} {key}"),
            Key::ActorModifier(id, key) => write!(f, "modifier {id} {key}"),
            Key::ActorBusy(id) => write!(f, "actor_busy {id}"),
            Key::ActorKnownFact(id) => write!(f, "actor_known_fact {id}"),
            Key::LastActRound(id) => write!(f, "last_act_round {id}"),
            Key::IdleActsThisWindow(id) => write!(f, "idle_acts_this_window {id}"),
            Key::LiveActsThisWindow(id) => write!(f, "live_acts_this_window {id}"),
            Key::ActorItem(actor, id) => write!(f, "actor_item {actor} {id}"),
            Key::PlaceItem(place, id) => write!(f, "place_item {place} {id}"),
            Key::PlaceMeter(place, key) => write!(f, "place_meter {place} {key}"),
            Key::PlaceFlag(place, key) => write!(f, "place_flag {place} {key}"),
            Key::StoryMeter(story, key) => write!(f, "story_meter {story} {key}"),
            Key::StoryFlag(story, key) => write!(f, "story_flag {story} {key}"),
            Key::ExitOpen(place, exit) => write!(f, "exit_open {place} {exit}"),
            Key::ActorWielded(actor) => write!(f, "actor_wielded {actor}"),
            Key::Note(id) => write!(f, "note {id}"),
            Key::WorldFlag(key) => write!(f, "world_flag {key}"),
            Key::WorldMeter(key) => write!(f, "world_meter {key}"),
        }
    }
}

impl Key {
    /// The key of flag `name` on `id`.
    pub(crate) fn flag(id: entity::IdRef<'_>, name: &str) -> Key {
        let name = name.to_string();
        match id {
            entity::IdRef::Actor(id) => Key::ActorFlag(id.to_string(), name),
            entity::IdRef::Place(id) => Key::PlaceFlag(id.to_string(), name),
            entity::IdRef::Story(id) => Key::StoryFlag(id.to_string(), name),
            entity::IdRef::Global => Key::WorldFlag(name),
        }
    }

    /// The key of meter `name` on `id`.
    pub(crate) fn meter(id: entity::IdRef<'_>, name: &str) -> Key {
        let name = name.to_string();
        match id {
            entity::IdRef::Actor(id) => Key::ActorMeter(id.to_string(), name),
            entity::IdRef::Place(id) => Key::PlaceMeter(id.to_string(), name),
            entity::IdRef::Story(id) => Key::StoryMeter(id.to_string(), name),
            entity::IdRef::Global => Key::WorldMeter(name),
        }
    }

    /// The actor this key belongs to, when it has one.
    pub fn actor(&self) -> Option<&str> {
        match self {
            Key::ActorPlace(id)
            | Key::ActorFlag(id, _)
            | Key::ActorMeter(id, _)
            | Key::ActorModifier(id, _)
            | Key::ActorBusy(id)
            | Key::ActorKnownFact(id)
            | Key::LastActRound(id)
            | Key::IdleActsThisWindow(id)
            | Key::LiveActsThisWindow(id)
            | Key::ActorItem(id, _)
            | Key::ActorWielded(id) => Some(id),
            Key::PlaceItem(_, _)
            | Key::PlaceMeter(_, _)
            | Key::PlaceFlag(_, _)
            | Key::StoryMeter(_, _)
            | Key::StoryFlag(_, _)
            | Key::ExitOpen(_, _)
            | Key::Note(_)
            | Key::WorldFlag(_)
            | Key::WorldMeter(_) => None,
        }
    }
}

/// One watched value. `Empty` means the key was absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Empty,
    Text(String),
    Number(i64),
    Bool(bool),
    List(Vec<String>),
}

impl Value {
    /// A meter's reading as a change value: `Empty` when it does not read.
    pub(crate) fn from_reading(value: Result<i64, crate::Reason>) -> Value {
        value.map_or(Value::Empty, Value::Number)
    }

    pub(crate) fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn as_number(&self) -> Option<i64> {
        match self {
            Value::Number(number) => Some(*number),
            _ => None,
        }
    }

    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(mark) => Some(*mark),
            _ => None,
        }
    }

    /// Whether this is a list that holds `id`.
    pub(crate) fn contains(&self, id: &str) -> bool {
        matches!(self, Value::List(ids) if ids.iter().any(|listed| listed == id))
    }
}

/// A key that changed, with the value before the change and the value after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub key: Key,
    pub before: Value,
    pub after: Value,
}

impl Change {
    pub(crate) fn new(key: Key, before: Value, after: Value) -> Change {
        Change { key, before, after }
    }
}

/// What a store's write reports: a finished change, or, for a meter, what the write replaced, so
/// the take can read the meter's change as rules read the meter.
#[derive(Clone, Debug)]
pub(crate) enum Report {
    Change(Change),
    /// A meter's write: the stored meter it replaced.
    Meter(Key, Option<Meter>),
    /// A modifier's write: what the actor's modifiers added to the meter before it.
    Modifiers(Key, Result<i64, Reason>),
}

impl Report {
    pub(crate) fn key(&self) -> &Key {
        match self {
            Self::Change(change) => &change.key,
            Self::Meter(key, _) | Self::Modifiers(key, _) => key,
        }
    }
}

impl From<Change> for Report {
    fn from(change: Change) -> Self {
        Self::Change(change)
    }
}
