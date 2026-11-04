//! One effect: its kind, and the fields that kind carries.

use serde::{Deserialize, Serialize};

use crate::draft::Draft;
use crate::intent::{Intent, Viewer};
use crate::model::key::StateKey;
use crate::model::meter::{self, MeterWrite};
use crate::model::{self, MemoryLine, Note};
use crate::rules::visibility::can_see_fact;
use crate::Reason;

/// One effect. JSON is `kind`, `as`, `intent_id`, then the fields of `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "EffectRecord", try_from = "EffectRecord")]
pub struct Effect {
    pub kind: EffectKind,
    pub viewer: Viewer,
    pub intent_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Meter {
        holder: Holder,
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
        holder: Holder,
        key: String,
        value: bool,
    },
    Note {
        note_id: String,
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

impl Effect {
    pub(crate) fn new(viewer: &Viewer, intent_id: &str, kind: EffectKind) -> Self {
        Effect {
            kind,
            viewer: viewer.clone(),
            intent_id: intent_id.to_string(),
        }
    }

    /// Write this effect into the staged save.
    pub(crate) fn apply(self, draft: &mut Draft, intent: &Intent) -> Result<(), Reason> {
        if !viewer_matches(intent, &self) {
            return Err(Reason::ViewerMismatch);
        }
        match self.kind {
            EffectKind::Meter { holder, key, write } => write_meter(draft, &holder, &key, write),
            EffectKind::Item {
                actor_id,
                place,
                id,
                delta,
                max,
            } => {
                let holder = match (actor_id, place) {
                    (Some(actor_id), None) => Holder::Actor(actor_id),
                    (None, Some(place)) => Holder::Place(place),
                    _ => return Err(Reason::Schema),
                };
                shift_item(draft, &holder, &id, delta, max)
            }
            EffectKind::Wield { actor_id, id } => wield(draft, &actor_id, &id),
            EffectKind::Unwield { actor_id } => unwield(draft, &actor_id),
            EffectKind::Location {
                actor_id, from, to, ..
            } => move_actor(draft, intent, &actor_id, &from, &to),
            EffectKind::Memory {
                actor_id,
                fact_ids,
                tags,
                text,
            } => remember(draft, intent, &actor_id, fact_ids, tags, text),
            EffectKind::Flag { holder, key, value } => set_flag(draft, &holder, &key, value),
            EffectKind::Note {
                note_id,
                to,
                place,
                fact_ids,
                text,
            } => leave_note(draft, note_id, to, place, fact_ids, text),
            EffectKind::Narrate { .. } => Ok(()),
            EffectKind::Reveal { actor_id, fact } => reveal(draft, &actor_id, &fact),
        }
    }
}

impl EffectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EffectKind::Meter { .. } => "meter",
            EffectKind::Item { .. } => "item",
            EffectKind::Wield { .. } => "wield",
            EffectKind::Unwield { .. } => "unwield",
            EffectKind::Location { .. } => "location",
            EffectKind::Memory { .. } => "memory",
            EffectKind::Flag { .. } => "flag",
            EffectKind::Note { .. } => "note",
            EffectKind::Narrate { .. } => "narrate",
            EffectKind::Reveal { .. } => "reveal",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EffectRecord {
    Meter {
        #[serde(rename = "as")]
        as_viewer: Viewer,
        intent_id: String,
        holder: Holder,
        key: String,
        #[serde(flatten)]
        write: MeterWrite,
    },
    Item {
        #[serde(rename = "as")]
        as_viewer: Viewer,
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
        as_viewer: Viewer,
        intent_id: String,
        actor_id: String,
        id: String,
    },
    Unwield {
        #[serde(rename = "as")]
        as_viewer: Viewer,
        intent_id: String,
        actor_id: String,
    },
    Location {
        #[serde(rename = "as")]
        as_viewer: Viewer,
        intent_id: String,
        actor_id: String,
        from: String,
        to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arrived_at: Option<u64>,
    },
    Memory {
        #[serde(rename = "as")]
        as_viewer: Viewer,
        intent_id: String,
        actor_id: String,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Flag {
        #[serde(rename = "as")]
        as_viewer: Viewer,
        intent_id: String,
        holder: Holder,
        key: String,
        value: bool,
    },
    Note {
        #[serde(rename = "as")]
        as_viewer: Viewer,
        intent_id: String,
        note_id: String,
        to: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        #[serde(rename = "as")]
        as_viewer: Viewer,
        intent_id: String,
        to: Vec<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Reveal {
        #[serde(rename = "as")]
        as_viewer: Viewer,
        intent_id: String,
        actor_id: String,
        fact: String,
    },
    #[serde(other)]
    Unknown,
}

impl From<Effect> for EffectRecord {
    fn from(effect: Effect) -> Self {
        let as_viewer = effect.viewer;
        let intent_id = effect.intent_id;
        match effect.kind {
            EffectKind::Meter { holder, key, write } => Self::Meter {
                as_viewer,
                intent_id,
                holder,
                key,
                write,
            },
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
            EffectKind::Flag { holder, key, value } => Self::Flag {
                as_viewer,
                intent_id,
                holder,
                key,
                value,
            },
            EffectKind::Note {
                note_id,
                to,
                place,
                fact_ids,
                text,
            } => Self::Note {
                as_viewer,
                intent_id,
                note_id,
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
        let (kind, as_viewer, intent_id) = match record {
            EffectRecord::Meter {
                as_viewer,
                intent_id,
                holder,
                key,
                write,
            } => (
                EffectKind::Meter { holder, key, write },
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
                holder,
                key,
                value,
            } => (
                EffectKind::Flag { holder, key, value },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Note {
                as_viewer,
                intent_id,
                note_id,
                to,
                place,
                fact_ids,
                text,
            } => (
                EffectKind::Note {
                    note_id,
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
            EffectRecord::Unknown => return Err(Reason::UnknownEffect),
        };
        Ok(Effect {
            kind,
            viewer: as_viewer,
            intent_id,
        })
    }
}

/// The effect's record minus `kind`, `as` and `intent_id`, keys in declaration order.
pub(crate) fn payload_json(effect: &Effect) -> String {
    let record = serde_json::to_value(effect).expect("effect payload");
    let serde_json::Value::Object(mut fields) = record else {
        unreachable!("effect records are objects")
    };
    for key in ["kind", "as", "intent_id"] {
        fields.shift_remove(key);
    }
    serde_json::to_string(&fields).expect("effect payload")
}

fn viewer_matches(intent: &Intent, effect: &Effect) -> bool {
    effect.viewer == intent.actor
}

/// Who holds a meter, a flag or an item count: an actor, a place, or the world.
/// Saved as `world`, `{actor: id}` or `{place: id}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Holder {
    Actor(String),
    Place(String),
    World,
}

/// Write a meter `holder` carries, inside the bounds the pack gives it.
pub(crate) fn write_meter(
    draft: &mut Draft,
    holder: &Holder,
    key: &str,
    write: MeterWrite,
) -> Result<(), Reason> {
    let def = draft
        .canon()
        .meter_def(holder, key)
        .cloned()
        .ok_or(Reason::UnknownId)?;
    let state_key = match holder {
        Holder::Actor(id) => StateKey::ActorMeter(id.clone(), key.to_string()),
        Holder::World => StateKey::WorldMeter(key.to_string()),
        Holder::Place(_) => return Err(Reason::UnknownId),
    };
    draft.mutate(state_key, |state| {
        let meters = state.meters_mut(holder)?;
        meter::write(&def, meters.get_mut(key).ok_or(Reason::UnknownId)?, write)
    })
}

/// Add `delta` to the count `holder` has of item `id`. An actor who no longer holds the item
/// they wield lets go of it.
pub(crate) fn shift_item(
    draft: &mut Draft,
    holder: &Holder,
    id: &str,
    delta: i64,
    max: Option<i64>,
) -> Result<(), Reason> {
    match holder {
        Holder::Actor(actor_id) => {
            let state_key = StateKey::ActorItem(actor_id.clone(), id.to_string());
            draft.mutate(state_key, |state| {
                model::shift_count(&mut state.actor_mut(actor_id)?.inventory, id, delta, max)
            })?;
            draft.mutate(StateKey::ActorWielded(actor_id.clone()), |state| {
                Ok(state.actor_mut(actor_id)?.release_wielded())
            })
        }
        Holder::Place(place) => {
            let state_key = StateKey::PlaceItem(place.clone(), id.to_string());
            draft.mutate(state_key, |state| {
                let place_live = state.places.get_mut(place).ok_or(Reason::UnknownId)?;
                model::shift_count(&mut place_live.items, id, delta, max)
            })
        }
        Holder::World => Err(Reason::Schema),
    }
}

fn wield(draft: &mut Draft, actor_id: &str, item_id: &str) -> Result<(), Reason> {
    let item = draft.canon().items.get(item_id).ok_or(Reason::UnknownId)?;
    if !item.tags.iter().any(|tag| tag == "weapon") {
        return Err(Reason::NotAWeapon);
    }
    draft.mutate(StateKey::ActorWielded(actor_id.to_string()), |state| {
        let actor = state.actor_mut(actor_id)?;
        if actor.item_count(item_id) == 0 {
            return Err(Reason::MissingItem);
        }
        let wielded = actor.wielded.replace(item_id.to_string());
        Ok(wielded.as_deref() != Some(item_id))
    })
}

fn unwield(draft: &mut Draft, actor_id: &str) -> Result<(), Reason> {
    draft.mutate(StateKey::ActorWielded(actor_id.to_string()), |state| {
        Ok(state.actor_mut(actor_id)?.wielded.take().is_some())
    })
}

fn move_actor(
    draft: &mut Draft,
    intent: &Intent,
    actor_id: &str,
    from: &str,
    to: &str,
) -> Result<(), Reason> {
    if actor_id != intent.actor.actor_id() {
        return Err(Reason::ActorMismatch);
    }
    let table_empty = draft.presence.players == 0;
    let in_canon = draft.canon().places.contains_key(to);
    draft.mutate(StateKey::ActorPlace(actor_id.to_string()), |state| {
        let actor = state.actors.get_mut(actor_id).ok_or(Reason::UnknownId)?;
        if table_empty && actor.is_pc() {
            return Err(Reason::TableEmpty);
        }
        if actor.place != from {
            return Err(Reason::OutOfRange);
        }
        if !in_canon && !state.places.contains_key(to) {
            return Err(Reason::NoExit);
        }
        let moved = actor.place != to;
        actor.place = to.to_string();
        actor.visit(to);
        // A canon place with no live record yet gets an empty one on the first arrival.
        state.places.entry(to.to_string()).or_default();
        Ok(moved)
    })
}

fn remember(
    draft: &mut Draft,
    intent: &Intent,
    actor_id: &str,
    mut fact_ids: Vec<String>,
    mut tags: Vec<String>,
    text: Option<String>,
) -> Result<(), Reason> {
    if actor_id != intent.actor.actor_id() {
        return Err(Reason::ActorMismatch);
    }
    if text.as_ref().is_some_and(|text| text.chars().count() > 240) {
        return Err(Reason::Schema);
    }
    for fact in &fact_ids {
        if !can_see_fact(draft.canon(), draft.state(), &intent.actor, fact) {
            return Err(Reason::IllegalKnowledge);
        }
    }
    draft.count_memory_write()?;
    model::sort_unique(&mut fact_ids);
    draft.mutate(StateKey::ActorKnownFact(actor_id.to_string()), |state| {
        let known = &mut state.actor_mut(actor_id)?.known_facts;
        let learned = fact_ids.iter().any(|fact| !known.contains(fact));
        if learned {
            known.extend(fact_ids.iter().cloned());
            model::sort_unique(known);
        }
        Ok(learned)
    })?;
    model::sort_unique(&mut tags);
    draft.push_memory(MemoryLine {
        round_index: draft.state().round_index,
        actor_id: actor_id.to_string(),
        intent_id: intent.id.clone(),
        fact_ids,
        tags,
        text,
    });
    Ok(())
}

/// Set or clear a flag the actor's cast file lists, or any world flag but `busy`.
fn set_flag(draft: &mut Draft, holder: &Holder, key: &str, value: bool) -> Result<(), Reason> {
    let flag_key = match holder {
        Holder::World if !key.is_empty() && key != "busy" => StateKey::WorldFlag(key.to_string()),
        Holder::Actor(id) => {
            let cast = draft.canon().cast.get(id);
            if !cast.is_some_and(|cast| cast.flags.iter().any(|flag| flag == key)) {
                return Err(Reason::UnknownId);
            }
            StateKey::ActorFlag(id.clone(), key.to_string())
        }
        _ => return Err(Reason::UnknownId),
    };
    draft.mutate(flag_key, |state| {
        let flags = match holder {
            Holder::Actor(id) => &mut state.actor_mut(id)?.flags,
            _ => &mut state.flags,
        };
        let had = flags.iter().any(|flag| flag == key);
        if !value {
            flags.retain(|flag| flag != key);
        } else if !had {
            flags.push(key.to_string());
        }
        Ok(had != value)
    })
}

fn reveal(draft: &mut Draft, actor_id: &str, fact: &str) -> Result<(), Reason> {
    if !draft.canon().facts.contains_key(fact) {
        return Err(Reason::UnknownId);
    }
    draft.mutate(StateKey::ActorKnownFact(actor_id.to_string()), |state| {
        let known = &mut state.actor_mut(actor_id)?.known_facts;
        if known.iter().any(|id| id == fact) {
            return Ok(false);
        }
        known.push(fact.to_string());
        model::sort_unique(known);
        Ok(true)
    })
}

fn leave_note(
    draft: &mut Draft,
    note_id: String,
    mut to: Vec<String>,
    place: Option<String>,
    mut fact_ids: Vec<String>,
    text: String,
) -> Result<(), Reason> {
    draft.mutate(StateKey::Note(note_id.clone()), |state| {
        if state.notes.iter().any(|note| note.id == note_id) {
            return Err(Reason::UnknownId);
        }
        if let Some(place_id) = &place {
            if !state.places.contains_key(place_id) {
                return Err(Reason::UnknownId);
            }
        }
        model::sort_unique(&mut to);
        model::sort_unique(&mut fact_ids);
        state.notes.push(Note {
            id: note_id,
            to,
            place,
            fact_ids,
            text,
        });
        Ok(true)
    })
}
