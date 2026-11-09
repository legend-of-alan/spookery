//! One effect: its kind, and the fields that kind carries.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::intent::{Intent, Viewer};
use crate::rules::visibility::can_see_fact;
use crate::state::id::{ActorId, EntityId, EntityRef, ExitId, PlaceId};
use crate::state::meter::MeterWrite;
use crate::state::modifier::{self, Modifier};
use crate::state::{self, nested_map, ActorKind, MemoryLine, Note, Presence, State};
use crate::Reason;

/// One effect. JSON is `kind`, `as`, `intent_id`, then the fields of `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "EffectRecord", try_from = "EffectRecord")]
pub struct Effect {
    pub kind: EffectKind,
    /// Who the effect was written as, or `None` for a change rule on a world change.
    pub viewer: Option<Viewer>,
    pub intent_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Meter {
        entity: EntityId,
        key: String,
        write: MeterWrite,
    },
    Item {
        actor_id: Option<ActorId>,
        place: Option<PlaceId>,
        id: String,
        delta: i64,
        max: Option<i64>,
    },
    /// One application of modifier `id`, of magnitude `add`, to an actor, lasting `ticks`.
    Modifier {
        actor_id: ActorId,
        id: String,
        add: i64,
        ticks: u64,
    },
    Wield {
        actor_id: ActorId,
        id: String,
    },
    Unwield {
        actor_id: ActorId,
    },
    Location {
        actor_id: ActorId,
        from: String,
        to: String,
        /// Exact tick of a timed arrival. Instant moves leave this empty.
        arrived_at: Option<u64>,
    },
    Memory {
        actor_id: ActorId,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        text: Option<String>,
    },
    Flag {
        entity: EntityId,
        key: String,
        value: bool,
    },
    /// Open or shut the door of an exit.
    ExitOpen {
        exit: ExitId,
        open: bool,
    },
    Note {
        note_id: String,
        to: Vec<ActorId>,
        place: Option<PlaceId>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        to: Vec<ActorId>,
        fact_ids: Vec<String>,
        text: String,
    },
    Reveal {
        actor_id: ActorId,
        fact: String,
    },
}

impl Effect {
    pub(crate) fn new(viewer: Option<&Viewer>, intent_id: &str, kind: EffectKind) -> Self {
        Effect {
            kind,
            viewer: viewer.cloned(),
            intent_id: intent_id.to_string(),
        }
    }

    /// Write this effect into the round's working copy of the save, with `presence` seated.
    pub(crate) fn apply(
        self,
        canon: &Canon,
        state: &mut State,
        presence: &Presence,
        intent: &Intent,
    ) -> Result<(), Reason> {
        if !viewer_matches(intent, &self) {
            return Err(Reason::ViewerMismatch);
        }
        match self.kind {
            EffectKind::Meter { entity, key, write } => {
                let MeterWrite::Offset(delta) = write;
                state.entity_mut(&entity).shift_meter(canon, &key, delta)
            }
            EffectKind::Item {
                actor_id,
                place,
                id,
                delta,
                max,
            } => {
                let entity = match (actor_id, place) {
                    (Some(actor_id), None) => EntityId::Actor(actor_id),
                    (None, Some(place)) => EntityId::Place(place),
                    _ => return Err(Reason::Schema),
                };
                state.entity_mut(&entity).shift_item(canon, &id, delta, max)
            }
            EffectKind::Modifier {
                actor_id,
                id,
                add,
                ticks,
            } => apply_modifier(canon, state, &actor_id, &id, add, ticks),
            EffectKind::Wield { actor_id, id } => {
                let item = canon.items.get(&id).ok_or(Reason::UnknownId)?;
                if !item.tags.iter().any(|tag| tag == "weapon") {
                    return Err(Reason::NotAWeapon);
                }
                state.actors_mut().set_wielded(&actor_id, Some(&id))
            }
            EffectKind::Unwield { actor_id } => state.actors_mut().set_wielded(&actor_id, None),
            EffectKind::Location {
                actor_id, from, to, ..
            } => move_actor(canon, state, presence, intent, &actor_id, &from, &to),
            EffectKind::Memory {
                actor_id,
                fact_ids,
                tags,
                text,
            } => remember(canon, state, intent, &actor_id, fact_ids, tags, text),
            EffectKind::Flag { entity, key, value } => {
                if !canon.carries_flag(entity.as_ref(), &key) {
                    return Err(Reason::UnknownId);
                }
                state.entity_mut(&entity).set_flag(&key, value)
            }
            // An exit with no `if` has no door.
            EffectKind::ExitOpen { exit, open } => {
                canon
                    .has_door(&exit)
                    .then_some(())
                    .ok_or(Reason::UnknownId)?;
                state.places_mut().set_exit_open(&exit, open)
            }
            EffectKind::Note {
                note_id,
                to,
                place,
                fact_ids,
                text,
            } => {
                if place
                    .as_ref()
                    .is_some_and(|at| !canon.places.contains_key(at))
                {
                    return Err(Reason::UnknownId);
                }
                let id = note_id;
                let note = Note {
                    id,
                    to,
                    place,
                    fact_ids,
                    text,
                };
                state.notes_mut().leave(note)
            }
            EffectKind::Narrate { .. } => Ok(()),
            EffectKind::Reveal { actor_id, fact } => {
                if !canon.facts.contains_key(&fact) {
                    return Err(Reason::UnknownId);
                }
                state.actors_mut().learn(&actor_id, &[fact])
            }
        }
    }
}

impl EffectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EffectKind::Meter { .. } => "meter",
            EffectKind::Item { .. } => "item",
            EffectKind::Modifier { .. } => "modifier",
            EffectKind::Wield { .. } => "wield",
            EffectKind::Unwield { .. } => "unwield",
            EffectKind::Location { .. } => "location",
            EffectKind::Memory { .. } => "memory",
            EffectKind::Flag { .. } => "flag",
            EffectKind::ExitOpen { .. } => "exit_open",
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
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        entity: EntityId,
        key: String,
        #[serde(flatten)]
        write: MeterWrite,
    },
    Item {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor_id: Option<ActorId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<PlaceId>,
        id: String,
        delta: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    Modifier {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: ActorId,
        id: String,
        add: i64,
        ticks: u64,
    },
    Wield {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: ActorId,
        id: String,
    },
    Unwield {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: ActorId,
    },
    Location {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: ActorId,
        from: String,
        to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arrived_at: Option<u64>,
    },
    Memory {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: ActorId,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Flag {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        entity: EntityId,
        key: String,
        value: bool,
    },
    ExitOpen {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        place: PlaceId,
        exit: String,
        open: bool,
    },
    Note {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        note_id: String,
        to: Vec<ActorId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<PlaceId>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        to: Vec<ActorId>,
        fact_ids: Vec<String>,
        text: String,
    },
    Reveal {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: ActorId,
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
            EffectKind::Meter { entity, key, write } => Self::Meter {
                as_viewer,
                intent_id,
                entity,
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
            EffectKind::Modifier {
                actor_id,
                id,
                add,
                ticks,
            } => Self::Modifier {
                as_viewer,
                intent_id,
                actor_id,
                id,
                add,
                ticks,
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
            EffectKind::Flag { entity, key, value } => Self::Flag {
                as_viewer,
                intent_id,
                entity,
                key,
                value,
            },
            EffectKind::ExitOpen { exit, open } => Self::ExitOpen {
                as_viewer,
                intent_id,
                place: exit.place,
                exit: exit.exit,
                open,
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
                entity,
                key,
                write,
            } => (
                EffectKind::Meter { entity, key, write },
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
            EffectRecord::Modifier {
                as_viewer,
                intent_id,
                actor_id,
                id,
                add,
                ticks,
            } => (
                EffectKind::Modifier {
                    actor_id,
                    id,
                    add,
                    ticks,
                },
                as_viewer,
                intent_id,
            ),
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
                entity,
                key,
                value,
            } => (
                EffectKind::Flag { entity, key, value },
                as_viewer,
                intent_id,
            ),
            EffectRecord::ExitOpen {
                as_viewer,
                intent_id,
                place,
                exit,
                open,
            } => {
                let exit = ExitId { place, exit };
                (EffectKind::ExitOpen { exit, open }, as_viewer, intent_id)
            }
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

/// Apply one stack of modifier `id` to an actor that carries its meter, lasting `ticks`, as
/// `modifier::stack` says.
fn apply_modifier(
    canon: &Canon,
    state: &mut State,
    actor_id: &ActorId,
    id: &str,
    add: i64,
    ticks: u64,
) -> Result<(), Reason> {
    let def = canon.modifiers.get(id).ok_or(Reason::UnknownId)?;
    let carried = canon.meter_def(EntityRef::Actor(actor_id), &def.meter);
    carried.ok_or(Reason::UnknownId)?;
    let now = state.world_time;
    let held = nested_map::get(&state.actors().modifiers, actor_id, id);
    let mut next = held.cloned().unwrap_or_else(|| Modifier {
        meter: def.meter.clone(),
        until: now,
        max_ticks: 0,
        stacks: Vec::new(),
    });
    modifier::stack(def, &mut next, add, ticks, now);
    state
        .actors_mut()
        .set_modifier(canon, actor_id, id, Some(next))
}

/// Move the intent's own actor from `from` to `to`, a place or the exit they walk. A PC moves
/// only while a player is seated.
fn move_actor(
    canon: &Canon,
    state: &mut State,
    presence: &Presence,
    intent: &Intent,
    actor_id: &ActorId,
    from: &str,
    to: &str,
) -> Result<(), Reason> {
    if intent.actor_id() != Some(actor_id) {
        return Err(Reason::ActorMismatch);
    }
    let actors = state.actors();
    if presence.players == 0 && actors.kind.get(actor_id) == Some(&ActorKind::Pc) {
        return Err(Reason::TableEmpty);
    }
    // The place is read to compare with `from`; an actor the save lacks has none, and is refused
    // here, before the canon's checks, as `set_place` would refuse it.
    if actors.place.get(actor_id).ok_or(Reason::UnknownId)? != from {
        return Err(Reason::OutOfRange);
    }
    // A walker stands on the exit until they arrive. The exit is no place of its own.
    let on_exit = crate::rules::travel::exit_stood_on(canon, to).is_some();
    if !canon.places.contains_key(to) && !on_exit {
        return Err(Reason::NoExit);
    }
    state.actors_mut().set_place(actor_id, to)
}

fn remember(
    canon: &Canon,
    state: &mut State,
    intent: &Intent,
    actor_id: &ActorId,
    mut fact_ids: Vec<String>,
    mut tags: Vec<String>,
    text: Option<String>,
) -> Result<(), Reason> {
    let Some(viewer) = intent
        .actor
        .as_ref()
        .filter(|viewer| viewer.actor_id() == actor_id)
    else {
        return Err(Reason::ActorMismatch);
    };
    if text.as_ref().is_some_and(|text| text.chars().count() > 240) {
        return Err(Reason::Schema);
    }
    for fact in &fact_ids {
        if !can_see_fact(canon, state, viewer, fact) {
            return Err(Reason::IllegalKnowledge);
        }
    }
    state.count_memory_write()?;
    state::sort_unique(&mut fact_ids);
    state.actors_mut().learn(actor_id, &fact_ids)?;
    state::sort_unique(&mut tags);
    state.push_memory(MemoryLine {
        round_index: state.round_index,
        actor_id: actor_id.clone(),
        intent_id: intent.id.clone(),
        fact_ids,
        tags,
        text,
    });
    Ok(())
}
