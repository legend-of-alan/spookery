//! One effect: its kind, and the fields that kind carries.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::draft::Draft;
use crate::intent::{Intent, Viewer};
use crate::model::meter::{self, MeterLive, MeterWrite};
use crate::model::modifier::{self, ModifierLive};
use crate::model::record::{ActorId, NotesId, PlaceId, StoryId, WorldFlagsId, WorldMetersId};
use crate::model::{self, MemoryLine, Note};
use crate::rules::visibility::can_see_fact;
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
    /// One application of modifier `id`, of magnitude `add`, to an actor, lasting `ticks`.
    Modifier {
        actor_id: String,
        id: String,
        add: i64,
        ticks: u64,
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
        /// Exact tick of a timed arrival. Instant moves leave this empty.
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
    pub(crate) fn new(viewer: Option<&Viewer>, intent_id: &str, kind: EffectKind) -> Self {
        Effect {
            kind,
            viewer: viewer.cloned(),
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
            EffectKind::Modifier {
                actor_id,
                id,
                add,
                ticks,
            } => apply_modifier(draft, &actor_id, &id, add, ticks),
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
            EffectKind::Modifier { .. } => "modifier",
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
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        holder: Holder,
        key: String,
        #[serde(flatten)]
        write: MeterWrite,
    },
    Item {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
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
    Modifier {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
        id: String,
        add: i64,
        ticks: u64,
    },
    Wield {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
        id: String,
    },
    Unwield {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
    },
    Location {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
        from: String,
        to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arrived_at: Option<u64>,
    },
    Memory {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Flag {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        holder: Holder,
        key: String,
        value: bool,
    },
    Note {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        note_id: String,
        to: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        to: Vec<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Reveal {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
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

/// Who holds a meter, a flag or an item count: an actor, a place, a story, an exit, or the
/// world. Saved as `world`, `{actor: id}`, `{place: id}`, `{story: id}` or
/// `{exit: [place, exit]}`. An exit holds only its flag `open`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Holder {
    Actor(String),
    Place(String),
    Story(String),
    Exit(String, String),
    World,
}

/// The holder as a rule's `who` names it: an actor id, `place:<id>`, `story:<id>`,
/// `exit:<place>.<exit>` or `world`.
impl fmt::Display for Holder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Holder::Actor(id) => write!(f, "{id}"),
            Holder::Place(id) => write!(f, "place:{id}"),
            Holder::Story(id) => write!(f, "story:{id}"),
            Holder::Exit(place, exit) => write!(f, "exit:{place}.{exit}"),
            Holder::World => write!(f, "world"),
        }
    }
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
    let write_live = |meters: &mut BTreeMap<String, MeterLive>| {
        let live = meters.get_mut(key).ok_or(Reason::UnknownId)?;
        meter::write(&def, live, write).map(drop)
    };
    match holder {
        Holder::Actor(id) => draft.write(ActorId(id), |actor| write_live(&mut actor.meters)),
        Holder::Place(id) => draft.write(PlaceId(id), |place| write_live(&mut place.meters)),
        Holder::Story(id) => draft.write(StoryId(id), |story| write_live(&mut story.meters)),
        Holder::World => draft.write(WorldMetersId, write_live),
        Holder::Exit(..) => Err(Reason::UnknownId),
    }
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
        Holder::Actor(actor_id) => draft.write(ActorId(actor_id), |actor| {
            model::shift_count(&mut actor.inventory, id, delta, max)?;
            actor.release_wielded();
            Ok(())
        }),
        Holder::Place(place) if draft.state().places.contains_key(place) => draft
            .write(PlaceId(place), |place| {
                model::shift_count(&mut place.items, id, delta, max).map(drop)
            }),
        Holder::Place(_) => Err(Reason::UnknownId),
        Holder::Story(_) | Holder::Exit(..) | Holder::World => Err(Reason::Schema),
    }
}

/// Apply one stack of modifier `id` to an actor that carries its meter, lasting `ticks`, as
/// `modifier::stack` says. The meter's value moves with the stacks while its fields hold.
fn apply_modifier(
    draft: &mut Draft,
    actor_id: &str,
    id: &str,
    add: i64,
    ticks: u64,
) -> Result<(), Reason> {
    let canon = draft.shared_canon();
    let def = canon.modifiers.get(id).ok_or(Reason::UnknownId)?;
    let meter_def = canon
        .meter_def(&Holder::Actor(actor_id.to_string()), &def.meter)
        .ok_or(Reason::UnknownId)?;
    let now = draft.state().world_time;
    draft.write(ActorId(actor_id), |actor| {
        let mut next = actor
            .modifiers
            .get(id)
            .cloned()
            .unwrap_or_else(|| ModifierLive {
                meter: def.meter.clone(),
                until: now,
                max_ticks: 0,
                stacks: Vec::new(),
            });
        modifier::stack(def, &mut next, add, ticks, now);
        actor.modifiers.insert(id.to_string(), next);
        // The actor must carry the meter, and the stacks must still sum to a value.
        let live = actor.meters.get(&def.meter).ok_or(Reason::UnknownId)?;
        meter::current(
            meter_def,
            live,
            modifier::sum(&actor.modifiers, &def.meter)?,
        )?;
        Ok(())
    })
}

fn wield(draft: &mut Draft, actor_id: &str, item_id: &str) -> Result<(), Reason> {
    let item = draft.canon().items.get(item_id).ok_or(Reason::UnknownId)?;
    if !item.tags.iter().any(|tag| tag == "weapon") {
        return Err(Reason::NotAWeapon);
    }
    draft.write(ActorId(actor_id), |actor| {
        if actor.item_count(item_id) == 0 {
            return Err(Reason::MissingItem);
        }
        actor.wielded = Some(item_id.to_string());
        Ok(())
    })
}

fn unwield(draft: &mut Draft, actor_id: &str) -> Result<(), Reason> {
    draft.write(ActorId(actor_id), |actor| {
        actor.wielded = None;
        Ok(())
    })
}

fn move_actor(
    draft: &mut Draft,
    intent: &Intent,
    actor_id: &str,
    from: &str,
    to: &str,
) -> Result<(), Reason> {
    if intent.actor_id() != Some(actor_id) {
        return Err(Reason::ActorMismatch);
    }
    let table_empty = draft.presence.players == 0;
    let in_canon = draft.canon().places.contains_key(to);
    // A walker stands on the exit until they arrive. The exit is no place of its own.
    let on_exit = crate::rules::travel::exit_stood_on(draft.canon(), to).is_some();
    let is_live = draft.state().places.contains_key(to);
    draft.write(ActorId(actor_id), |actor| {
        if table_empty && actor.is_pc() {
            return Err(Reason::TableEmpty);
        }
        if actor.place != from {
            return Err(Reason::OutOfRange);
        }
        if !in_canon && !on_exit && !is_live {
            return Err(Reason::NoExit);
        }
        actor.place = to.to_string();
        Ok(())
    })?;
    // A canon place with no live record yet gets an empty one on the first arrival.
    if !on_exit {
        draft.write(PlaceId(to), |_| Ok(()))?;
    }
    Ok(())
}

fn remember(
    draft: &mut Draft,
    intent: &Intent,
    actor_id: &str,
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
        if !can_see_fact(draft.canon(), draft.state(), viewer, fact) {
            return Err(Reason::IllegalKnowledge);
        }
    }
    draft.count_memory_write()?;
    model::sort_unique(&mut fact_ids);
    draft.write(ActorId(actor_id), |actor| {
        actor.known_facts.extend(fact_ids.iter().cloned());
        model::sort_unique(&mut actor.known_facts);
        Ok(())
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

/// Set or clear a flag `holder` carries. An exit's `open` is its name in its place's list.
fn set_flag(draft: &mut Draft, holder: &Holder, key: &str, value: bool) -> Result<(), Reason> {
    if !draft.canon().carries_flag(holder, key) {
        return Err(Reason::UnknownId);
    }
    let set = |flags: &mut Vec<String>, entry: &str| {
        if value && !flags.iter().any(|flag| flag == entry) {
            flags.push(entry.to_string());
        } else if !value {
            flags.retain(|flag| flag != entry);
        }
        Ok(())
    };
    match holder {
        Holder::Actor(id) => draft.write(ActorId(id), |actor| set(&mut actor.flags, key)),
        Holder::Place(id) => draft.write(PlaceId(id), |place| set(&mut place.flags, key)),
        Holder::Exit(place, exit) => {
            draft.write(PlaceId(place), |place| set(&mut place.open, exit))
        }
        Holder::Story(id) => draft.write(StoryId(id), |story| set(&mut story.flags, key)),
        Holder::World => draft.write(WorldFlagsId, |flags| set(flags, key)),
    }
}

fn reveal(draft: &mut Draft, actor_id: &str, fact: &str) -> Result<(), Reason> {
    if !draft.canon().facts.contains_key(fact) {
        return Err(Reason::UnknownId);
    }
    draft.write(ActorId(actor_id), |actor| {
        actor.known_facts.push(fact.to_string());
        model::sort_unique(&mut actor.known_facts);
        Ok(())
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
    if draft.state().notes.iter().any(|note| note.id == note_id) {
        return Err(Reason::UnknownId);
    }
    if let Some(place_id) = &place {
        if !draft.state().places.contains_key(place_id) {
            return Err(Reason::UnknownId);
        }
    }
    model::sort_unique(&mut to);
    model::sort_unique(&mut fact_ids);
    draft.write(NotesId, |notes| {
        notes.push(Note {
            id: note_id,
            to,
            place,
            fact_ids,
            text,
        });
        Ok(())
    })
}
