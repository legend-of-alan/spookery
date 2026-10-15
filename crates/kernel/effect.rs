use serde::{Deserialize, Serialize};

use crate::intent::ViewerId;
use crate::meter::MeterWrite;

/// One effect. JSON is `class`, `as`, `intent_id`, then the fields of `class`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "crate::store::records::EffectRecord",
    into = "crate::store::records::EffectRecord"
)]
pub struct Effect {
    pub class: EffectClass,
    pub as_viewer: ViewerId,
    pub intent_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectClass {
    Clock {
        clock_id: String,
        delta: i64,
        /// Clear `paused` before applying `delta`.
        start: bool,
    },
    Meter {
        actor_id: String,
        key: String,
        write: MeterWrite,
    },
    Item {
        actor_id: Option<String>,
        place: Option<String>,
        id: String,
        delta: i64,
        max: Option<i64>,
    },
    Wield {
        actor_id: String,
        id: String,
    },
    Unwield {
        actor_id: String,
    },
    Location {
        actor_id: String,
        from: String,
        to: String,
        /// Exact world minute of a timed arrival. Instant moves leave this empty.
        arrived_at: Option<u64>,
    },
    Memory {
        actor_id: String,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        text: Option<String>,
    },
    Flag {
        actor_id: String,
        key: String,
        value: bool,
    },
    Message {
        message_id: String,
        to: Vec<String>,
        place: Option<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        to: Vec<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Reveal {
        actor_id: String,
        fact: String,
    },
}

/// Actor id on a flag effect that names the world, not a person.
pub const WORLD: &str = "world";

impl Effect {
    pub fn as_viewer(&self) -> &ViewerId {
        &self.as_viewer
    }

    pub fn intent_id(&self) -> &str {
        &self.intent_id
    }

    pub fn class_name(&self) -> &'static str {
        self.class.name()
    }
}

impl EffectClass {
    pub fn name(&self) -> &'static str {
        match self {
            EffectClass::Clock { .. } => "clock",
            EffectClass::Meter { .. } => "meter",
            EffectClass::Item { .. } => "item",
            EffectClass::Wield { .. } => "wield",
            EffectClass::Unwield { .. } => "unwield",
            EffectClass::Location { .. } => "location",
            EffectClass::Memory { .. } => "memory",
            EffectClass::Flag { .. } => "flag",
            EffectClass::Message { .. } => "message",
            EffectClass::Narrate { .. } => "narrate",
            EffectClass::Reveal { .. } => "reveal",
        }
    }
}
