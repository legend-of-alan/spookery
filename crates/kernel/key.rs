use std::fmt;

/// One budget identity. The commit diff summary is this type's `Display`,
/// sorted as text. It is not a path inside `save.yaml`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StateKey {
    ActorPlace(String),
    ActorFlag(String, String),
    ActorMeter(String, String),
    ActorBusy(String),
    ActorKnownFact(String),
    TickLastAct(String),
    TickIdleActs(String),
    TickLiveActs(String),
    ActorItem(String, String),
    PlaceItem(String, String),
    ActorWielded(String),
    ClockValue(String),
    ClockPaused(String),
    Message(String),
    WorldFlag(String),
}

impl fmt::Display for StateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateKey::ActorPlace(id) => write!(f, "actor_place {id}"),
            StateKey::ActorFlag(id, key) => write!(f, "actor_flag {id} {key}"),
            StateKey::ActorMeter(id, key) => write!(f, "actor_meter {id} {key}"),
            StateKey::ActorBusy(id) => write!(f, "actor_busy {id}"),
            StateKey::ActorKnownFact(id) => write!(f, "actor_known_fact {id}"),
            StateKey::TickLastAct(id) => write!(f, "tick_last_act {id}"),
            StateKey::TickIdleActs(id) => write!(f, "tick_idle_acts {id}"),
            StateKey::TickLiveActs(id) => write!(f, "tick_live_acts {id}"),
            StateKey::ActorItem(actor, id) => write!(f, "actor_item {actor} {id}"),
            StateKey::PlaceItem(place, id) => write!(f, "place_item {place} {id}"),
            StateKey::ActorWielded(actor) => write!(f, "actor_wielded {actor}"),
            StateKey::ClockValue(id) => write!(f, "clock_value {id}"),
            StateKey::ClockPaused(id) => write!(f, "clock_paused {id}"),
            StateKey::Message(id) => write!(f, "message {id}"),
            StateKey::WorldFlag(key) => write!(f, "world_flag {key}"),
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
            | StateKey::ActorBusy(id)
            | StateKey::ActorKnownFact(id)
            | StateKey::TickLastAct(id)
            | StateKey::TickIdleActs(id)
            | StateKey::TickLiveActs(id)
            | StateKey::ActorItem(id, _)
            | StateKey::ActorWielded(id) => Some(id),
            StateKey::PlaceItem(_, _)
            | StateKey::ClockValue(_)
            | StateKey::ClockPaused(_)
            | StateKey::Message(_)
            | StateKey::WorldFlag(_) => None,
        }
    }
}

/// One watched value. `Empty` means the key was absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateValue {
    Empty,
    Text(String),
    Number(i64),
    Mark(bool),
    List(Vec<String>),
}

/// A key that changed, with the value before the change and the value after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeRecord {
    pub key: StateKey,
    pub before: StateValue,
    pub after: StateValue,
}
