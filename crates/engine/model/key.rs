//! State keys a commit watches, and the values they held.

use std::fmt;

/// One budget identity. The commit diff summary is this type's `Display`,
/// sorted as text. It is not a path inside `save.yaml`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StateKey {
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

impl fmt::Display for StateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateKey::ActorPlace(id) => write!(f, "actor_place {id}"),
            StateKey::ActorFlag(id, key) => write!(f, "actor_flag {id} {key}"),
            StateKey::ActorMeter(id, key) => write!(f, "actor_meter {id} {key}"),
            StateKey::ActorModifier(id, key) => write!(f, "modifier {id} {key}"),
            StateKey::ActorBusy(id) => write!(f, "actor_busy {id}"),
            StateKey::ActorKnownFact(id) => write!(f, "actor_known_fact {id}"),
            StateKey::LastActRound(id) => write!(f, "last_act_round {id}"),
            StateKey::IdleActsThisWindow(id) => write!(f, "idle_acts_this_window {id}"),
            StateKey::LiveActsThisWindow(id) => write!(f, "live_acts_this_window {id}"),
            StateKey::ActorItem(actor, id) => write!(f, "actor_item {actor} {id}"),
            StateKey::PlaceItem(place, id) => write!(f, "place_item {place} {id}"),
            StateKey::PlaceMeter(place, key) => write!(f, "place_meter {place} {key}"),
            StateKey::PlaceFlag(place, key) => write!(f, "place_flag {place} {key}"),
            StateKey::StoryMeter(story, key) => write!(f, "story_meter {story} {key}"),
            StateKey::StoryFlag(story, key) => write!(f, "story_flag {story} {key}"),
            StateKey::ExitOpen(place, exit) => write!(f, "exit_open {place} {exit}"),
            StateKey::ActorWielded(actor) => write!(f, "actor_wielded {actor}"),
            StateKey::Note(id) => write!(f, "note {id}"),
            StateKey::WorldFlag(key) => write!(f, "world_flag {key}"),
            StateKey::WorldMeter(key) => write!(f, "world_meter {key}"),
        }
    }
}

impl StateKey {
    /// The actor this key belongs to, when it has one.
    pub fn actor(&self) -> Option<&str> {
        match self {
            StateKey::ActorPlace(id)
            | StateKey::ActorFlag(id, _)
            | StateKey::ActorMeter(id, _)
            | StateKey::ActorModifier(id, _)
            | StateKey::ActorBusy(id)
            | StateKey::ActorKnownFact(id)
            | StateKey::LastActRound(id)
            | StateKey::IdleActsThisWindow(id)
            | StateKey::LiveActsThisWindow(id)
            | StateKey::ActorItem(id, _)
            | StateKey::ActorWielded(id) => Some(id),
            StateKey::PlaceItem(_, _)
            | StateKey::PlaceMeter(_, _)
            | StateKey::PlaceFlag(_, _)
            | StateKey::StoryMeter(_, _)
            | StateKey::StoryFlag(_, _)
            | StateKey::ExitOpen(_, _)
            | StateKey::Note(_)
            | StateKey::WorldFlag(_)
            | StateKey::WorldMeter(_) => None,
        }
    }
}

/// One watched value. `Empty` means the key was absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateValue {
    Empty,
    Text(String),
    Number(i64),
    Bool(bool),
    List(Vec<String>),
}

/// How each kind of stored field reads as a value: the one place that says so, for a read
/// of the record now and for a pre-image alike.
impl From<&String> for StateValue {
    fn from(text: &String) -> Self {
        StateValue::Text(text.clone())
    }
}

impl From<bool> for StateValue {
    fn from(mark: bool) -> Self {
        StateValue::Bool(mark)
    }
}

impl From<i64> for StateValue {
    fn from(number: i64) -> Self {
        StateValue::Number(number)
    }
}

impl From<u32> for StateValue {
    fn from(count: u32) -> Self {
        StateValue::Number(i64::from(count))
    }
}

/// A tick or round count past `i64::MAX` reads as `i64::MAX`.
impl From<u64> for StateValue {
    fn from(count: u64) -> Self {
        StateValue::Number(i64::try_from(count).unwrap_or(i64::MAX))
    }
}

impl From<&Vec<String>> for StateValue {
    fn from(ids: &Vec<String>) -> Self {
        StateValue::List(ids.clone())
    }
}

/// An absent field or record reads `Empty`.
impl<T: Into<StateValue>> From<Option<T>> for StateValue {
    fn from(held: Option<T>) -> Self {
        held.map_or(StateValue::Empty, Into::into)
    }
}

impl StateValue {
    pub(crate) fn as_text(&self) -> Option<&str> {
        match self {
            StateValue::Text(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn as_number(&self) -> Option<i64> {
        match self {
            StateValue::Number(number) => Some(*number),
            _ => None,
        }
    }

    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            StateValue::Bool(mark) => Some(*mark),
            _ => None,
        }
    }

    /// Whether this is a list that holds `id`.
    pub(crate) fn contains(&self, id: &str) -> bool {
        matches!(self, StateValue::List(ids) if ids.iter().any(|listed| listed == id))
    }
}

/// A key that changed, with the value before the change and the value after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub key: StateKey,
    pub before: StateValue,
    pub after: StateValue,
}
