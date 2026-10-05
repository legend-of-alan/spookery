use serde::ser::{Serialize, SerializeStruct, Serializer};

use crate::intent::ViewerId;

/// One effect. `class` is the payload. JSON for this value is written by the
/// serializer below: `class`, `as`, `intent_id`, then the fields of `class`.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    },
    Need {
        actor_id: String,
        key: String,
        delta: i64,
    },
    Stock {
        place: String,
        sku: String,
        delta: i64,
    },
    Location {
        actor_id: String,
        from: String,
        to: String,
    },
    Relationship {
        a: String,
        b: String,
        delta: i64,
    },
    Reputation {
        actor_id: String,
        faction: String,
        delta: i64,
    },
    Memory {
        actor_id: String,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        text: Option<String>,
    },
    Rumor {
        rumor_id: String,
        fact_ids: Vec<String>,
        from: String,
        to: String,
        heat: i64,
    },
    Resource {
        actor_id: String,
        key: String,
        delta: i64,
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
}

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
            EffectClass::Need { .. } => "need",
            EffectClass::Stock { .. } => "stock",
            EffectClass::Location { .. } => "location",
            EffectClass::Relationship { .. } => "relationship",
            EffectClass::Reputation { .. } => "reputation",
            EffectClass::Memory { .. } => "memory",
            EffectClass::Rumor { .. } => "rumor",
            EffectClass::Resource { .. } => "resource",
            EffectClass::Flag { .. } => "flag",
            EffectClass::Message { .. } => "message",
        }
    }
}

impl Serialize for Effect {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let viewer = self.as_viewer.token();
        let mut state = serializer.serialize_struct("Effect", 3 + payload_len(&self.class))?;
        state.serialize_field("class", self.class.name())?;
        state.serialize_field("as", &viewer)?;
        state.serialize_field("intent_id", &self.intent_id)?;
        write_payload_fields(&mut state, &self.class)?;
        state.end()
    }
}

/// Payload object only. Digest rows print this, not the envelope.
pub(crate) struct EffectPayload<'a>(pub &'a EffectClass);

impl Serialize for EffectPayload<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("EffectPayload", payload_len(self.0))?;
        write_payload_fields(&mut state, self.0)?;
        state.end()
    }
}

fn payload_len(class: &EffectClass) -> usize {
    match class {
        EffectClass::Clock { .. } => 2,
        EffectClass::Need { .. }
        | EffectClass::Stock { .. }
        | EffectClass::Location { .. }
        | EffectClass::Relationship { .. }
        | EffectClass::Reputation { .. }
        | EffectClass::Resource { .. }
        | EffectClass::Flag { .. } => 3,
        EffectClass::Memory { text, .. } => 3 + usize::from(text.is_some()),
        EffectClass::Rumor { .. } => 5,
        EffectClass::Message { place, .. } => 4 + usize::from(place.is_some()),
    }
}

fn write_payload_fields<S>(state: &mut S, class: &EffectClass) -> Result<(), S::Error>
where
    S: SerializeStruct,
{
    match class {
        EffectClass::Clock { clock_id, delta } => {
            state.serialize_field("clock_id", clock_id)?;
            state.serialize_field("delta", delta)?;
        }
        EffectClass::Need {
            actor_id,
            key,
            delta,
        }
        | EffectClass::Resource {
            actor_id,
            key,
            delta,
        } => {
            state.serialize_field("actor_id", actor_id)?;
            state.serialize_field("key", key)?;
            state.serialize_field("delta", delta)?;
        }
        EffectClass::Stock { place, sku, delta } => {
            state.serialize_field("place", place)?;
            state.serialize_field("sku", sku)?;
            state.serialize_field("delta", delta)?;
        }
        EffectClass::Location { actor_id, from, to } => {
            state.serialize_field("actor_id", actor_id)?;
            state.serialize_field("from", from)?;
            state.serialize_field("to", to)?;
        }
        EffectClass::Relationship { a, b, delta } => {
            state.serialize_field("a", a)?;
            state.serialize_field("b", b)?;
            state.serialize_field("delta", delta)?;
        }
        EffectClass::Reputation {
            actor_id,
            faction,
            delta,
        } => {
            state.serialize_field("actor_id", actor_id)?;
            state.serialize_field("faction", faction)?;
            state.serialize_field("delta", delta)?;
        }
        EffectClass::Memory {
            actor_id,
            fact_ids,
            tags,
            text,
        } => {
            state.serialize_field("actor_id", actor_id)?;
            state.serialize_field("fact_ids", fact_ids)?;
            state.serialize_field("tags", tags)?;
            if let Some(text) = text {
                state.serialize_field("text", text)?;
            }
        }
        EffectClass::Rumor {
            rumor_id,
            fact_ids,
            from,
            to,
            heat,
        } => {
            state.serialize_field("rumor_id", rumor_id)?;
            state.serialize_field("fact_ids", fact_ids)?;
            state.serialize_field("from", from)?;
            state.serialize_field("to", to)?;
            state.serialize_field("heat", heat)?;
        }
        EffectClass::Flag {
            actor_id,
            key,
            value,
        } => {
            state.serialize_field("actor_id", actor_id)?;
            state.serialize_field("key", key)?;
            state.serialize_field("value", value)?;
        }
        EffectClass::Message {
            message_id,
            to,
            place,
            fact_ids,
            text,
        } => {
            state.serialize_field("message_id", message_id)?;
            state.serialize_field("to", to)?;
            if let Some(place) = place {
                state.serialize_field("place", place)?;
            }
            state.serialize_field("fact_ids", fact_ids)?;
            state.serialize_field("text", text)?;
        }
    }
    Ok(())
}
