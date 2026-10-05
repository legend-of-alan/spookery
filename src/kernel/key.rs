use std::fmt;

/// One budget identity. The commit diff summary is this type's `Display`,
/// sorted as text. It is not a path inside `now.yaml`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StateKey {
    ActorPlace(String),
    ActorFlag(String, String),
    ActorNeed(String, String),
    ActorResource(String, String),
    ActorBusy(String),
    ActorKnownFact(String),
    TickLastAct(String),
    TickIdleActs(String),
    TickLiveActs(String),
    PlaceStock(String, String),
    ClockValue(String),
    ClockPaused(String),
    Relationship(String, String),
    Reputation(String, String),
    Rumor(String),
    Message(String),
}

impl fmt::Display for StateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateKey::ActorPlace(id) => write!(f, "actor_place {id}"),
            StateKey::ActorFlag(id, key) => write!(f, "actor_flag {id} {key}"),
            StateKey::ActorNeed(id, key) => write!(f, "actor_need {id} {key}"),
            StateKey::ActorResource(id, key) => write!(f, "actor_resource {id} {key}"),
            StateKey::ActorBusy(id) => write!(f, "actor_busy {id}"),
            StateKey::ActorKnownFact(id) => write!(f, "actor_known_fact {id}"),
            StateKey::TickLastAct(id) => write!(f, "tick_last_act {id}"),
            StateKey::TickIdleActs(id) => write!(f, "tick_idle_acts {id}"),
            StateKey::TickLiveActs(id) => write!(f, "tick_live_acts {id}"),
            StateKey::PlaceStock(place, sku) => write!(f, "place_stock {place} {sku}"),
            StateKey::ClockValue(id) => write!(f, "clock_value {id}"),
            StateKey::ClockPaused(id) => write!(f, "clock_paused {id}"),
            StateKey::Relationship(a, b) => write!(f, "relationship {a} {b}"),
            StateKey::Reputation(actor, faction) => write!(f, "reputation {actor} {faction}"),
            StateKey::Rumor(id) => write!(f, "rumor {id}"),
            StateKey::Message(id) => write!(f, "message {id}"),
        }
    }
}
