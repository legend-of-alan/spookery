//! One effect: its class, and the fields that class carries.

use serde::{Deserialize, Serialize};

use crate::error::Reason;
use crate::intent::ViewerId;
use crate::meter::MeterWrite;

/// One effect. JSON is `class`, `as`, `intent_id`, then the fields of `class`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "EffectRecord")]
pub struct Effect {
    pub class: EffectKind,
    pub as_viewer: ViewerId,
    pub intent_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectKind {
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

    pub(crate) fn class_name(&self) -> &'static str {
        self.class.name()
    }
}

impl EffectKind {
    pub fn name(&self) -> &'static str {
        match self {
            EffectKind::Clock { .. } => "clock",
            EffectKind::Meter { .. } => "meter",
            EffectKind::Item { .. } => "item",
            EffectKind::Wield { .. } => "wield",
            EffectKind::Unwield { .. } => "unwield",
            EffectKind::Location { .. } => "location",
            EffectKind::Memory { .. } => "memory",
            EffectKind::Flag { .. } => "flag",
            EffectKind::Message { .. } => "message",
            EffectKind::Narrate { .. } => "narrate",
            EffectKind::Reveal { .. } => "reveal",
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WriteKind {
    Base,
    Offset,
    Snap,
}

fn absent<T>(value: &Option<T>) -> bool {
    value.is_none()
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "class", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EffectRecord {
    Clock {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        clock_id: String,
        delta: i64,
        #[serde(default, skip_serializing_if = "is_false")]
        start: bool,
    },
    Meter {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        actor_id: String,
        key: String,
        write: WriteKind,
        #[serde(default, skip_serializing_if = "absent")]
        delta: Option<i64>,
    },
    Item {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        #[serde(default, skip_serializing_if = "absent")]
        actor_id: Option<String>,
        #[serde(default, skip_serializing_if = "absent")]
        place: Option<String>,
        id: String,
        delta: i64,
        #[serde(default, skip_serializing_if = "absent")]
        max: Option<i64>,
    },
    Wield {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        actor_id: String,
        id: String,
    },
    Unwield {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        actor_id: String,
    },
    Location {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        actor_id: String,
        from: String,
        to: String,
        #[serde(default, skip_serializing_if = "absent")]
        arrived_at: Option<u64>,
    },
    Memory {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        actor_id: String,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "absent")]
        text: Option<String>,
    },
    Flag {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        actor_id: String,
        key: String,
        value: bool,
    },
    Message {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        message_id: String,
        to: Vec<String>,
        #[serde(default, skip_serializing_if = "absent")]
        place: Option<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        to: Vec<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Reveal {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        actor_id: String,
        fact: String,
    },
    #[serde(other)]
    Illegal,
}

impl From<Effect> for EffectRecord {
    fn from(effect: Effect) -> Self {
        let as_viewer = effect.as_viewer;
        let intent_id = effect.intent_id;
        match effect.class {
            EffectKind::Clock {
                clock_id,
                delta,
                start,
            } => Self::Clock {
                as_viewer,
                intent_id,
                clock_id,
                delta,
                start,
            },
            EffectKind::Meter {
                actor_id,
                key,
                write,
            } => {
                let (write, delta) = match write {
                    MeterWrite::Base(delta) => (WriteKind::Base, Some(delta)),
                    MeterWrite::Offset(delta) => (WriteKind::Offset, Some(delta)),
                    MeterWrite::Snap => (WriteKind::Snap, None),
                };
                Self::Meter {
                    as_viewer,
                    intent_id,
                    actor_id,
                    key,
                    write,
                    delta,
                }
            }
            EffectKind::Item {
                actor_id,
                place,
                id,
                delta,
                max,
            } => Self::Item {
                as_viewer,
                intent_id,
                actor_id,
                place,
                id,
                delta,
                max,
            },
            EffectKind::Wield { actor_id, id } => Self::Wield {
                as_viewer,
                intent_id,
                actor_id,
                id,
            },
            EffectKind::Unwield { actor_id } => Self::Unwield {
                as_viewer,
                intent_id,
                actor_id,
            },
            EffectKind::Location {
                actor_id,
                from,
                to,
                arrived_at,
            } => Self::Location {
                as_viewer,
                intent_id,
                actor_id,
                from,
                to,
                arrived_at,
            },
            EffectKind::Memory {
                actor_id,
                fact_ids,
                tags,
                text,
            } => Self::Memory {
                as_viewer,
                intent_id,
                actor_id,
                fact_ids,
                tags,
                text,
            },
            EffectKind::Flag {
                actor_id,
                key,
                value,
            } => Self::Flag {
                as_viewer,
                intent_id,
                actor_id,
                key,
                value,
            },
            EffectKind::Message {
                message_id,
                to,
                place,
                fact_ids,
                text,
            } => Self::Message {
                as_viewer,
                intent_id,
                message_id,
                to,
                place,
                fact_ids,
                text,
            },
            EffectKind::Narrate { to, fact_ids, text } => Self::Narrate {
                as_viewer,
                intent_id,
                to,
                fact_ids,
                text,
            },
            EffectKind::Reveal { actor_id, fact } => Self::Reveal {
                as_viewer,
                intent_id,
                actor_id,
                fact,
            },
        }
    }
}

impl TryFrom<EffectRecord> for Effect {
    type Error = Reason;

    fn try_from(record: EffectRecord) -> Result<Self, Self::Error> {
        let (class, as_viewer, intent_id) = match record {
            EffectRecord::Clock {
                as_viewer,
                intent_id,
                clock_id,
                delta,
                start,
            } => (
                EffectKind::Clock {
                    clock_id,
                    delta,
                    start,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Meter {
                as_viewer,
                intent_id,
                actor_id,
                key,
                write,
                delta,
            } => (
                EffectKind::Meter {
                    actor_id,
                    key,
                    write: meter_write(write, delta)?,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Item {
                as_viewer,
                intent_id,
                actor_id,
                place,
                id,
                delta,
                max,
            } => {
                if actor_id.is_some() == place.is_some() {
                    return Err(Reason::Schema);
                }
                (
                    EffectKind::Item {
                        actor_id,
                        place,
                        id,
                        delta,
                        max,
                    },
                    as_viewer,
                    intent_id,
                )
            }
            EffectRecord::Wield {
                as_viewer,
                intent_id,
                actor_id,
                id,
            } => (EffectKind::Wield { actor_id, id }, as_viewer, intent_id),
            EffectRecord::Unwield {
                as_viewer,
                intent_id,
                actor_id,
            } => (EffectKind::Unwield { actor_id }, as_viewer, intent_id),
            EffectRecord::Location {
                as_viewer,
                intent_id,
                actor_id,
                from,
                to,
                arrived_at,
            } => (
                EffectKind::Location {
                    actor_id,
                    from,
                    to,
                    arrived_at,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Memory {
                as_viewer,
                intent_id,
                actor_id,
                fact_ids,
                tags,
                text,
            } => (
                EffectKind::Memory {
                    actor_id,
                    fact_ids,
                    tags,
                    text,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Flag {
                as_viewer,
                intent_id,
                actor_id,
                key,
                value,
            } => (
                EffectKind::Flag {
                    actor_id,
                    key,
                    value,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Message {
                as_viewer,
                intent_id,
                message_id,
                to,
                place,
                fact_ids,
                text,
            } => (
                EffectKind::Message {
                    message_id,
                    to,
                    place,
                    fact_ids,
                    text,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Narrate {
                as_viewer,
                intent_id,
                to,
                fact_ids,
                text,
            } => (
                EffectKind::Narrate { to, fact_ids, text },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Reveal {
                as_viewer,
                intent_id,
                actor_id,
                fact,
            } => (EffectKind::Reveal { actor_id, fact }, as_viewer, intent_id),
            EffectRecord::Illegal => return Err(Reason::IllegalClass),
        };
        Ok(Effect {
            class,
            as_viewer,
            intent_id,
        })
    }
}

fn meter_write(write: WriteKind, delta: Option<i64>) -> Result<MeterWrite, Reason> {
    match (write, delta) {
        (WriteKind::Snap, None) => Ok(MeterWrite::Snap),
        (WriteKind::Base, Some(delta)) => Ok(MeterWrite::Base(delta)),
        (WriteKind::Offset, Some(delta)) => Ok(MeterWrite::Offset(delta)),
        (WriteKind::Snap, Some(_)) | (WriteKind::Base | WriteKind::Offset, None) => {
            Err(Reason::Schema)
        }
    }
}

/// The effect's record minus `class`, `as` and `intent_id`, keys in declaration order.
pub(crate) fn payload_json(effect: &Effect) -> String {
    let record = serde_json::to_value(effect).expect("effect payload");
    let serde_json::Value::Object(mut fields) = record else {
        unreachable!("effect records are objects")
    };
    for key in ["class", "as", "intent_id"] {
        fields.shift_remove(key);
    }
    serde_json::to_string(&fields).expect("effect payload")
}
