//! State keys a commit watches, and the values they held.

use std::fmt;

use crate::rules::effect::Holder;

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
    /// The key of meter `key` on `holder`. An exit carries no meters.
    pub(crate) fn meter(holder: &Holder, key: &str) -> Option<StateKey> {
        let key = key.to_string();
        match holder {
            Holder::Actor(id) => Some(StateKey::ActorMeter(id.clone(), key)),
            Holder::Place(id) => Some(StateKey::PlaceMeter(id.clone(), key)),
            Holder::Story(id) => Some(StateKey::StoryMeter(id.clone(), key)),
            Holder::Exit(..) => None,
            Holder::World => Some(StateKey::WorldMeter(key)),
        }
    }

    /// The key of item `id`'s count on `holder`. Only an actor or a place holds items.
    pub(crate) fn item(holder: &Holder, id: &str) -> Option<StateKey> {
        let id = id.to_string();
        match holder {
            Holder::Actor(actor) => Some(StateKey::ActorItem(actor.clone(), id)),
            Holder::Place(place) => Some(StateKey::PlaceItem(place.clone(), id)),
            Holder::Story(_) | Holder::Exit(..) | Holder::World => None,
        }
    }

    /// The key of flag `key` on `holder`. An exit's one flag is `open`.
    pub(crate) fn flag(holder: &Holder, key: &str) -> StateKey {
        let key = key.to_string();
        match holder {
            Holder::Exit(place, exit) => StateKey::ExitOpen(place.clone(), exit.clone()),
            Holder::Actor(id) => StateKey::ActorFlag(id.clone(), key),
            Holder::Place(id) => StateKey::PlaceFlag(id.clone(), key),
            Holder::Story(id) => StateKey::StoryFlag(id.clone(), key),
            Holder::World => StateKey::WorldFlag(key),
        }
    }

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
