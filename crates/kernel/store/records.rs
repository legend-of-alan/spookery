//! Journal wire shapes. Field order is the on-disk key order.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::effect::{Effect, EffectClass};
use crate::intent::{IntentArgs, Target, ViewerId};
use crate::meter::MeterWrite;

#[derive(Debug)]
pub(crate) struct RecordFault {
    illegal_class: bool,
    field: &'static str,
}

impl RecordFault {
    fn class() -> Self {
        Self {
            illegal_class: true,
            field: "class",
        }
    }

    fn field(field: &'static str) -> Self {
        Self {
            illegal_class: false,
            field,
        }
    }
}

impl fmt::Display for RecordFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.illegal_class {
            f.write_str("ILLEGAL_CLASS")
        } else {
            f.write_str(self.field)
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
    #[serde(other)]
    Illegal,
}

impl From<Effect> for EffectRecord {
    fn from(effect: Effect) -> Self {
        let as_viewer = effect.as_viewer;
        let intent_id = effect.intent_id;
        match effect.class {
            EffectClass::Clock {
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
            EffectClass::Meter {
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
            EffectClass::Item {
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
            EffectClass::Wield { actor_id, id } => Self::Wield {
                as_viewer,
                intent_id,
                actor_id,
                id,
            },
            EffectClass::Unwield { actor_id } => Self::Unwield {
                as_viewer,
                intent_id,
                actor_id,
            },
            EffectClass::Location {
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
            EffectClass::Memory {
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
            EffectClass::Flag {
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
            EffectClass::Message {
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
            EffectClass::Narrate { to, fact_ids, text } => Self::Narrate {
                as_viewer,
                intent_id,
                to,
                fact_ids,
                text,
            },
        }
    }
}

impl TryFrom<EffectRecord> for Effect {
    type Error = RecordFault;

    fn try_from(record: EffectRecord) -> Result<Self, Self::Error> {
        let (class, as_viewer, intent_id) = match record {
            EffectRecord::Clock {
                as_viewer,
                intent_id,
                clock_id,
                delta,
                start,
            } => (
                EffectClass::Clock {
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
                EffectClass::Meter {
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
                    return Err(RecordFault::field("actor_id"));
                }
                (
                    EffectClass::Item {
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
            } => (EffectClass::Wield { actor_id, id }, as_viewer, intent_id),
            EffectRecord::Unwield {
                as_viewer,
                intent_id,
                actor_id,
            } => (EffectClass::Unwield { actor_id }, as_viewer, intent_id),
            EffectRecord::Location {
                as_viewer,
                intent_id,
                actor_id,
                from,
                to,
                arrived_at,
            } => (
                EffectClass::Location {
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
                EffectClass::Memory {
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
                EffectClass::Flag {
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
                EffectClass::Message {
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
                EffectClass::Narrate { to, fact_ids, text },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Illegal => return Err(RecordFault::class()),
        };
        Ok(Effect {
            class,
            as_viewer,
            intent_id,
        })
    }
}

fn meter_write(write: WriteKind, delta: Option<i64>) -> Result<MeterWrite, RecordFault> {
    match (write, delta) {
        (WriteKind::Snap, None) => Ok(MeterWrite::Snap),
        (WriteKind::Base, Some(delta)) => Ok(MeterWrite::Base(delta)),
        (WriteKind::Offset, Some(delta)) => Ok(MeterWrite::Offset(delta)),
        (WriteKind::Snap, Some(_)) | (WriteKind::Base | WriteKind::Offset, None) => {
            Err(RecordFault::field("delta"))
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetWire {
    #[serde(default, skip_serializing_if = "absent")]
    place: Option<String>,
    #[serde(default, skip_serializing_if = "absent")]
    actor: Option<String>,
    #[serde(default, skip_serializing_if = "absent")]
    clock: Option<String>,
}

impl From<&Target> for TargetWire {
    fn from(target: &Target) -> Self {
        match target {
            Target::Empty => Self {
                place: None,
                actor: None,
                clock: None,
            },
            Target::Place(id) => Self {
                place: Some(id.clone()),
                actor: None,
                clock: None,
            },
            Target::Actor(id) => Self {
                place: None,
                actor: Some(id.clone()),
                clock: None,
            },
            Target::Clock(id) => Self {
                place: None,
                actor: None,
                clock: Some(id.clone()),
            },
        }
    }
}

impl TryFrom<TargetWire> for Target {
    type Error = &'static str;

    fn try_from(wire: TargetWire) -> Result<Self, Self::Error> {
        match (wire.place, wire.actor, wire.clock) {
            (None, None, None) => Ok(Target::Empty),
            (Some(id), None, None) => Ok(Target::Place(id)),
            (None, Some(id), None) => Ok(Target::Actor(id)),
            (None, None, Some(id)) => Ok(Target::Clock(id)),
            _ => Err("target"),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArgsWire {
    #[serde(rename = "to", default, skip_serializing_if = "absent")]
    message_to: Option<Vec<String>>,
}

impl From<&IntentArgs> for ArgsWire {
    fn from(args: &IntentArgs) -> Self {
        Self {
            message_to: args.message_to.clone(),
        }
    }
}

impl From<ArgsWire> for IntentArgs {
    fn from(wire: ArgsWire) -> Self {
        Self {
            message_to: wire.message_to,
        }
    }
}

pub(crate) mod target_as {
    use super::TargetWire;
    use crate::intent::Target;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(target: &Target, serializer: S) -> Result<S::Ok, S::Error> {
        TargetWire::from(target).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Target, D::Error> {
        Target::try_from(TargetWire::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

pub(crate) mod args_as {
    use super::ArgsWire;
    use crate::intent::IntentArgs;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(args: &IntentArgs, serializer: S) -> Result<S::Ok, S::Error> {
        ArgsWire::from(args).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<IntentArgs, D::Error> {
        Ok(IntentArgs::from(ArgsWire::deserialize(deserializer)?))
    }
}

/// `need`, `resource`, and `inventory` stay [`crate::error::Reason::IllegalClass`].
pub(crate) fn effect_reason(message: &str) -> crate::error::Reason {
    if message.contains("ILLEGAL_CLASS") {
        crate::error::Reason::IllegalClass
    } else {
        crate::error::Reason::Schema
    }
}
