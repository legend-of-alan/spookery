//! One effect: its class, and the fields that class carries.

use serde::{Deserialize, Serialize};

use crate::draft::Draft;
use crate::error::Reason;
use crate::intent::{Intent, ViewerId};
use crate::key::{StateKey, StateValue};
use crate::meter::{self, MeterWrite};
use crate::model::{self, MemoryLine, Message, PlaceLive};
use crate::visibility::allows;

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
    pub(crate) fn new(viewer: &ViewerId, intent_id: &str, class: EffectKind) -> Self {
        Effect {
            class,
            as_viewer: viewer.clone(),
            intent_id: intent_id.to_string(),
        }
    }

    /// Write this effect into the staged save.
    pub(crate) fn apply(self, draft: &mut Draft, intent: &Intent) -> Result<(), Reason> {
        if !viewer_matches(intent, &self) {
            return Err(Reason::ViewerMismatch);
        }
        match self.class {
            EffectKind::Clock {
                clock_id,
                delta,
                start,
            } => apply_clock(draft, intent.actor.actor_id(), &clock_id, delta, start),
            EffectKind::Meter {
                actor_id,
                key,
                write,
            } => apply_meter(draft, &actor_id, &key, write),
            EffectKind::Item {
                actor_id,
                place,
                id,
                delta,
                max,
            } => apply_item(draft, actor_id, place, &id, delta, max),
            EffectKind::Wield { actor_id, id } => apply_wield(draft, &actor_id, &id),
            EffectKind::Unwield { actor_id } => apply_unwield(draft, &actor_id),
            EffectKind::Location {
                actor_id, from, to, ..
            } => apply_location(draft, intent, &actor_id, &from, &to),
            EffectKind::Memory {
                actor_id,
                fact_ids,
                tags,
                text,
            } => apply_memory(draft, intent, &actor_id, fact_ids, tags, text),
            EffectKind::Flag {
                actor_id,
                key,
                value,
            } => apply_flag(draft, &actor_id, &key, value),
            EffectKind::Message {
                message_id,
                to,
                place,
                fact_ids,
                text,
            } => apply_message(draft, message_id, to, place, fact_ids, text),
            EffectKind::Narrate { .. } => Ok(()),
            EffectKind::Reveal { actor_id, fact } => apply_reveal(draft, &actor_id, &fact),
        }
    }

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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta: Option<i64>,
    },
    Item {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<String>,
        id: String,
        delta: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arrived_at: Option<u64>,
    },
    Memory {
        #[serde(rename = "as")]
        as_viewer: ViewerId,
        intent_id: String,
        actor_id: String,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
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

fn viewer_matches(intent: &Intent, effect: &Effect) -> bool {
    effect.as_viewer() == &intent.actor
}

fn apply_clock(
    draft: &mut Draft,
    actor_id: &str,
    clock_id: &str,
    delta: i64,
    start: bool,
) -> Result<(), Reason> {
    let humans = draft.presence.humans;
    let clock = draft.now().clocks.get(clock_id).ok_or(Reason::UnknownKey)?;
    if !clock.owners.iter().any(|owner| owner == actor_id) {
        return Err(Reason::UnknownKey);
    }
    if start && clock.paused {
        draft.track(StateKey::ClockPaused(clock_id.to_string()))?;
        draft
            .now_mut()
            .clocks
            .get_mut(clock_id)
            .ok_or(Reason::UnknownKey)?
            .paused = false;
    }
    let clock = draft.now().clocks.get(clock_id).ok_or(Reason::UnknownKey)?;
    if delta == 0 && start {
        return Ok(());
    }
    if clock.paused || (clock.requires_presence && humans == 0) {
        return Err(Reason::PausedClock);
    }
    let current = clock.value;
    let next = current.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if next < 0 || next > clock.max {
        return Err(Reason::OutOfRange);
    }
    if next != current {
        draft.track(StateKey::ClockValue(clock_id.to_string()))?;
    }
    draft
        .now_mut()
        .clocks
        .get_mut(clock_id)
        .ok_or(Reason::UnknownKey)?
        .value = next;
    Ok(())
}

fn apply_meter(
    draft: &mut Draft,
    actor_id: &str,
    key: &str,
    write: MeterWrite,
) -> Result<(), Reason> {
    let def = *draft.canon().meters.get(key).ok_or(Reason::UnknownKey)?;
    let mut live = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .meters()
        .get(key)
        .cloned()
        .ok_or(Reason::UnknownKey)?;
    let changed = meter::write(&def, &mut live, write)?;
    if changed {
        draft.track(StateKey::ActorMeter(actor_id.to_string(), key.to_string()))?;
    }
    *draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .meters_mut()
        .get_mut(key)
        .ok_or(Reason::UnknownKey)? = live;
    Ok(())
}

fn apply_item(
    draft: &mut Draft,
    actor_id: Option<String>,
    place: Option<String>,
    id: &str,
    delta: i64,
    max: Option<i64>,
) -> Result<(), Reason> {
    match (actor_id, place) {
        (Some(actor_id), None) => {
            let before = i64::from(
                draft
                    .now()
                    .actors
                    .get(&actor_id)
                    .ok_or(Reason::UnknownKey)?
                    .count(id),
            );
            let wielded = draft
                .now()
                .actors
                .get(&actor_id)
                .ok_or(Reason::UnknownKey)?
                .wielded()
                .map(str::to_string);
            let changed = {
                let inventory = draft
                    .now_mut()
                    .actors
                    .get_mut(&actor_id)
                    .ok_or(Reason::UnknownKey)?
                    .inventory_mut();
                model::shift_count(inventory, id, delta, max)?
            };
            if changed {
                draft.track_before(
                    StateKey::ActorItem(actor_id.clone(), id.to_string()),
                    StateValue::Number(before),
                )?;
            }
            let released = draft
                .now_mut()
                .actors
                .get_mut(&actor_id)
                .ok_or(Reason::UnknownKey)?
                .release_wielded();
            if released {
                draft.track_before(
                    StateKey::ActorWielded(actor_id),
                    wielded.map(StateValue::Text).unwrap_or(StateValue::Empty),
                )?;
            }
            Ok(())
        }
        (None, Some(place)) => {
            let before = i64::from(
                draft
                    .now()
                    .places
                    .get(&place)
                    .ok_or(Reason::UnknownKey)?
                    .items
                    .get(id)
                    .copied()
                    .unwrap_or(0),
            );
            let changed = {
                let items = &mut draft
                    .now_mut()
                    .places
                    .get_mut(&place)
                    .ok_or(Reason::UnknownKey)?
                    .items;
                model::shift_count(items, id, delta, max)?
            };
            if changed {
                draft.track_before(
                    StateKey::PlaceItem(place, id.to_string()),
                    StateValue::Number(before),
                )?;
            }
            Ok(())
        }
        _ => Err(Reason::Schema),
    }
}

fn apply_wield(draft: &mut Draft, actor_id: &str, item_id: &str) -> Result<(), Reason> {
    let Some(item) = draft.canon().items.get(item_id) else {
        return Err(Reason::UnknownKey);
    };
    if !item.tags.iter().any(|tag| tag == "weapon") {
        return Err(Reason::NotAWeapon);
    }
    let held = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .count(item_id);
    if held == 0 {
        return Err(Reason::MissingItem);
    }
    let current = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .wielded()
        .map(str::to_string);
    if current.as_deref() == Some(item_id) {
        return Ok(());
    }
    draft.track(StateKey::ActorWielded(actor_id.to_string()))?;
    *draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .wielded_mut() = Some(item_id.to_string());
    Ok(())
}

fn apply_unwield(draft: &mut Draft, actor_id: &str) -> Result<(), Reason> {
    let set = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .wielded()
        .is_some();
    if !set {
        return Ok(());
    }
    draft.track(StateKey::ActorWielded(actor_id.to_string()))?;
    *draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .wielded_mut() = None;
    Ok(())
}

fn apply_location(
    draft: &mut Draft,
    intent: &Intent,
    actor_id: &str,
    from: &str,
    to: &str,
) -> Result<(), Reason> {
    if actor_id != intent.actor.actor_id() {
        return Err(Reason::ActorMismatch);
    }
    let actor = draft.now().actors.get(actor_id).ok_or(Reason::UnknownKey)?;
    if draft.presence.humans == 0 && actor.is_pc() {
        return Err(Reason::TableEmpty);
    }
    if actor.place() != from {
        return Err(Reason::OutOfRange);
    }
    if !draft.canon().places.contains_key(to) && !draft.now().places.contains_key(to) {
        return Err(Reason::NoExit);
    }
    let place_changed = actor.place() != to;
    if place_changed {
        draft.track(StateKey::ActorPlace(actor_id.to_string()))?;
    }
    let actor = draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?;
    *actor.place_mut() = to.to_string();
    actor.visit(to);
    if !draft.now().places.contains_key(to) {
        draft.now_mut().places.insert(
            to.to_string(),
            PlaceLive {
                items: std::collections::BTreeMap::new(),
                notes: Vec::new(),
                actors: Vec::new(),
            },
        );
    }
    Ok(())
}

fn apply_memory(
    draft: &mut Draft,
    intent: &Intent,
    actor_id: &str,
    fact_ids: Vec<String>,
    tags: Vec<String>,
    text: Option<String>,
) -> Result<(), Reason> {
    if actor_id != intent.actor.actor_id() {
        return Err(Reason::ActorMismatch);
    }
    if text.as_ref().is_some_and(|text| text.chars().count() > 240) {
        return Err(Reason::Schema);
    }
    for fact in &fact_ids {
        if !allows(draft.canon(), draft.now(), &intent.actor, fact) {
            return Err(Reason::IllegalKnowledge);
        }
    }
    draft.bump_memory()?;
    let mut fact_ids = fact_ids;
    fact_ids.sort();
    fact_ids.dedup();
    let known = draft
        .now()
        .actors
        .get(actor_id)
        .map(|actor| actor.known_facts().to_vec())
        .unwrap_or_default();
    if fact_ids
        .iter()
        .any(|fact| !known.iter().any(|id| id == fact))
    {
        draft.track(StateKey::ActorKnownFact(actor_id.to_string()))?;
        let facts = draft
            .now_mut()
            .actors
            .get_mut(actor_id)
            .ok_or(Reason::UnknownKey)?
            .known_facts_mut();
        for fact in &fact_ids {
            if !facts.iter().any(|id| id == fact) {
                facts.push(fact.clone());
            }
        }
        model::sort_unique(facts);
    }
    let mut tags = tags;
    tags.sort();
    tags.dedup();
    draft.stage_memory(MemoryLine {
        round_index: draft.now().round_index,
        actor_id: actor_id.to_string(),
        intent_id: intent.id.clone(),
        fact_ids,
        tags,
        text,
    });
    Ok(())
}

fn apply_flag(draft: &mut Draft, actor_id: &str, key: &str, value: bool) -> Result<(), Reason> {
    if actor_id == WORLD {
        return apply_world_flag(draft, key, value);
    }
    let listed = draft
        .canon()
        .cast
        .get(actor_id)
        .is_some_and(|cast| cast.flags.iter().any(|flag| flag == key));
    if !listed {
        return Err(Reason::UnknownKey);
    }
    let has = draft
        .now()
        .actors
        .get(actor_id)
        .is_some_and(|actor| match actor {
            crate::model::ActorLive::Npc { flags, .. }
            | crate::model::ActorLive::Pc { flags, .. } => flags.iter().any(|flag| flag == key),
        });
    if has != value {
        draft.track(StateKey::ActorFlag(actor_id.to_string(), key.to_string()))?;
    }
    let flags = draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .flags_mut();
    if value {
        if !flags.iter().any(|flag| flag == key) {
            flags.push(key.to_string());
        }
    } else {
        flags.retain(|flag| flag != key);
    }
    Ok(())
}

fn apply_world_flag(draft: &mut Draft, key: &str, value: bool) -> Result<(), Reason> {
    if key.is_empty() || key == "busy" {
        return Err(Reason::UnknownKey);
    }
    let has = draft.now().flags.iter().any(|flag| flag == key);
    if has != value {
        draft.track(StateKey::WorldFlag(key.to_string()))?;
    }
    let flags = &mut draft.now_mut().flags;
    if value {
        if !flags.iter().any(|flag| flag == key) {
            flags.push(key.to_string());
        }
    } else {
        flags.retain(|flag| flag != key);
    }
    Ok(())
}

fn apply_reveal(draft: &mut Draft, actor_id: &str, fact: &str) -> Result<(), Reason> {
    if !draft.canon().facts.contains_key(fact) {
        return Err(Reason::UnknownKey);
    }
    let known = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .known_facts();
    if known.iter().any(|id| id == fact) {
        return Ok(());
    }
    draft.track(StateKey::ActorKnownFact(actor_id.to_string()))?;
    let facts = draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .known_facts_mut();
    facts.push(fact.to_string());
    model::sort_unique(facts);
    Ok(())
}

fn apply_message(
    draft: &mut Draft,
    message_id: String,
    to: Vec<String>,
    place: Option<String>,
    fact_ids: Vec<String>,
    text: String,
) -> Result<(), Reason> {
    if draft
        .now()
        .messages
        .iter()
        .any(|message| message.id == message_id)
    {
        return Err(Reason::UnknownKey);
    }
    if let Some(place_id) = &place {
        if !draft.now().places.contains_key(place_id) {
            return Err(Reason::UnknownKey);
        }
    }
    draft.track(StateKey::Message(message_id.clone()))?;
    if let Some(place_id) = &place {
        let mut notes = draft
            .now()
            .places
            .get(place_id)
            .map(|place_live| place_live.notes.clone())
            .unwrap_or_default();
        notes.push(message_id.clone());
        notes.sort();
        notes.dedup();
        if let Some(place_live) = draft.now_mut().places.get_mut(place_id) {
            place_live.notes = notes;
        }
    }
    let mut to = to;
    to.sort();
    to.dedup();
    let mut fact_ids = fact_ids;
    fact_ids.sort();
    fact_ids.dedup();
    draft.now_mut().messages.push(Message {
        id: message_id,
        to,
        place,
        fact_ids,
        text,
    });
    Ok(())
}
